use std::{
    collections::BTreeMap,
    f64::consts::{FRAC_1_SQRT_2, PI},
};

use caditor_geometry::{Point2, Vector2};
use caditor_sketch::{ArcGeometry, Constraint, Entity, EntityId, Reference, Sketch};

use crate::snap::Screen;

const DIMENSION_OFFSET: f64 = 28.0;
const LANE_SPACING: f64 = 22.0;
const LANE_TOLERANCE: f64 = 1e-6;
const LANE_DIRECTION_TOLERANCE: f64 = 1e-6;
const EXTENSION_GAP: f64 = 4.0;
const EXTENSION_OVERSHOOT: f64 = 5.0;
const MIN_EXTENSION: f64 = 1.0;
const ARROW_LENGTH: f64 = 9.0;
const ANGLE_RADIUS: f64 = 44.0;
const RADIUS_OVERSHOOT: f64 = 24.0;
const ARC_STEP: f64 = PI / 36.0;
const MIN_ARC_SEGMENTS: f64 = 2.0;
const PARALLEL_SINE: f64 = 0.01;
const DEGENERATE: f64 = 1e-9;
const GLYPH_OFFSET: f64 = 14.0;
const GLYPH_SPACING: f64 = 18.0;
const POINT_GLYPH_OFFSET: Vector2 = Vector2::new(10.0, -10.0);
const POINT_GLYPH_SPACING: f64 = 12.0;
const CIRCLE_LEADER_DIRECTION: Vector2 = Vector2::new(FRAC_1_SQRT_2, FRAC_1_SQRT_2);
const CIRCLE_GLYPH_DIRECTION: Vector2 = Vector2::new(-FRAC_1_SQRT_2, -FRAC_1_SQRT_2);
const ARC_LEADER_FRACTION: f64 = 0.5;
const ARC_GLYPH_FRACTION: f64 = 0.25;
const SPLINE_GLYPH_ANGLE: f64 = PI / 18.0;
const GLYPH_SHIFT_STEP: f64 = GLYPH_SPACING / 2.0;
const MAX_CURVE_GLYPH_SHIFT: f64 = GLYPH_SPACING * 2.0;
const GLYPH_STEPS_BEYOND: usize = 4;
const MAX_GLYPH_SHIFTS: usize = 64;
const GLYPH_VIEW_MARGIN: f64 = GLYPH_OFFSET + GLYPH_SPACING;
const OBSTACLE_CELL: f64 = 32.0;
const MAX_OBSTACLE_CELLS: i64 = 64;
const POINT_GLYPH_QUADRANTS: [Vector2; 4] = [
    Vector2::ONE,
    Vector2::new(-1.0, 1.0),
    Vector2::new(1.0, -1.0),
    Vector2::NEG_ONE,
];

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LineSpan {
    origin: Point2,
    direction: Vector2,
    length: Option<f64>,
}

impl LineSpan {
    fn of(sketch: &Sketch, id: EntityId) -> Option<Self> {
        let infinite = |direction| Self {
            origin: Point2::ZERO,
            direction,
            length: None,
        };
        match id.reference() {
            Some(Reference::HorizontalAxis) => Some(infinite(Vector2::X)),
            Some(Reference::VerticalAxis) => Some(infinite(Vector2::Y)),
            Some(Reference::Origin) => None,
            None => {
                let (start, end) = sketch.line_endpoints(id)?;
                let length = start.distance(end);
                (length > DEGENERATE).then(|| Self {
                    origin: start,
                    direction: (end - start) / length,
                    length: Some(length),
                })
            }
        }
    }

    fn at(&self, parameter: f64) -> Point2 {
        self.origin + self.direction * parameter
    }

    fn parameter(&self, point: Point2) -> f64 {
        (point - self.origin).dot(self.direction)
    }

    fn foot(&self, point: Point2) -> Point2 {
        self.at(self.parameter(point))
    }

    fn clamped(&self, parameter: f64) -> f64 {
        self.length
            .map_or(parameter, |length| parameter.clamp(0.0, length))
    }

    fn closest(&self, point: Point2) -> Point2 {
        self.at(self.clamped(self.parameter(point)))
    }

    fn middle(&self) -> Option<Point2> {
        self.length.map(|length| self.at(length / 2.0))
    }

    fn ends(&self) -> Vec<Point2> {
        self.length
            .map(|length| vec![self.origin, self.at(length)])
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Measured {
    Points(Point2, Point2),
    Aligned {
        from: Point2,
        to: Point2,
        along: Vector2,
    },
    PointToLine(Point2, LineSpan),
    PointToCircle {
        point: Point2,
        center: Point2,
        radius: f64,
    },
    Angle(LineSpan, LineSpan, bool),
    Radius {
        center: Point2,
        radius: f64,
        toward: Vector2,
    },
    Diameter {
        center: Point2,
        radius: f64,
        toward: Vector2,
    },
    AlongArc {
        arc: ArcGeometry,
        from_centre: bool,
    },
}

pub fn measured(sketch: &Sketch, constraint: &Constraint) -> Option<Measured> {
    match *constraint {
        Constraint::Distance { from, to, .. } => match (sketch.point(from), sketch.point(to)) {
            (Some(a), Some(b)) => Some(Measured::Points(a, b)),
            (Some(point), None) => point_to_curve(sketch, point, to),
            (None, Some(point)) => point_to_curve(sketch, point, from),
            (None, None) => curve_to_curve(sketch, from, to),
        },
        Constraint::HorizontalDistance { from, to, .. } => Some(Measured::Aligned {
            from: sketch.point(from)?,
            to: sketch.point(to)?,
            along: Vector2::X,
        }),
        Constraint::VerticalDistance { from, to, .. } => Some(Measured::Aligned {
            from: sketch.point(from)?,
            to: sketch.point(to)?,
            along: Vector2::Y,
        }),
        Constraint::Angle {
            from, to, reversed, ..
        } => Some(Measured::Angle(
            LineSpan::of(sketch, from)?,
            LineSpan::of(sketch, to)?,
            reversed,
        )),
        Constraint::Radius { entity, .. } => {
            let (center, radius, toward) = leader(sketch, entity)?;
            Some(Measured::Radius {
                center,
                radius,
                toward,
            })
        }
        Constraint::Diameter { entity, .. } => {
            let (center, radius, toward) = leader(sketch, entity)?;
            Some(Measured::Diameter {
                center,
                radius,
                toward,
            })
        }
        Constraint::ArcLength { arc, .. } => Some(Measured::AlongArc {
            arc: sketch.arc(arc)?,
            from_centre: false,
        }),
        Constraint::Sweep { arc, .. } => Some(Measured::AlongArc {
            arc: sketch.arc(arc)?,
            from_centre: true,
        }),
        Constraint::Coincident(..)
        | Constraint::Horizontal(_)
        | Constraint::Vertical(_)
        | Constraint::HorizontalPoints(..)
        | Constraint::VerticalPoints(..)
        | Constraint::Parallel(..)
        | Constraint::Perpendicular(..)
        | Constraint::Tangent(..)
        | Constraint::Equal(..)
        | Constraint::Midpoint { .. }
        | Constraint::Concentric(..)
        | Constraint::Collinear(..)
        | Constraint::Symmetric { .. }
        | Constraint::Fix { .. } => None,
    }
}

fn point_to_curve(sketch: &Sketch, point: Point2, curve: EntityId) -> Option<Measured> {
    if let Some(line) = LineSpan::of(sketch, curve) {
        return Some(Measured::PointToLine(point, line));
    }
    let (center, radius) = sketch.circle(curve)?;
    Some(Measured::PointToCircle {
        point,
        center,
        radius,
    })
}

fn curve_to_curve(sketch: &Sketch, from: EntityId, to: EntityId) -> Option<Measured> {
    match (sketch.circle(from), sketch.circle(to)) {
        (None, None) => {
            let (anchor, other) = if to.is_reference() {
                (to, from)
            } else {
                (from, to)
            };
            let (start, end) = sketch.line_endpoints(other)?;
            Some(Measured::PointToLine(
                (start + end) / 2.0,
                LineSpan::of(sketch, anchor)?,
            ))
        }
        (Some(circle), None) => circle_to_line(circle, LineSpan::of(sketch, to)?),
        (None, Some(circle)) => circle_to_line(circle, LineSpan::of(sketch, from)?),
        (Some((first, first_radius)), Some((second, second_radius))) => {
            let (center, radius, inner, inner_radius) = if first_radius >= second_radius {
                (first, first_radius, second, second_radius)
            } else {
                (second, second_radius, first, first_radius)
            };
            let outward = (inner - center).try_normalize().unwrap_or(Vector2::X);
            let between = inner.distance(center);
            let near_side = if between > radius + inner_radius {
                -inner_radius
            } else {
                inner_radius
            };
            Some(Measured::PointToCircle {
                point: inner + outward * near_side,
                center,
                radius,
            })
        }
    }
}

fn circle_to_line((center, radius): (Point2, f64), line: LineSpan) -> Option<Measured> {
    let toward = (line.foot(center) - center)
        .try_normalize()
        .unwrap_or(line.direction.perp());
    Some(Measured::PointToLine(center + toward * radius, line))
}

fn leader(sketch: &Sketch, entity: EntityId) -> Option<(Point2, f64, Vector2)> {
    let (center, radius) = sketch.circle(entity)?;
    let toward = match sketch.arc(entity) {
        Some(arc) => Vector2::from_angle(arc.start_angle + arc.sweep * ARC_LEADER_FRACTION),
        None => CIRCLE_LEADER_DIRECTION,
    };
    Some((center, radius, toward))
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Arrow {
    pub tip: Vector2,
    pub direction: Vector2,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DimensionLayout {
    pub strokes: Vec<Vec<Vector2>>,
    pub arrows: Vec<Arrow>,
    pub label: Vector2,
    pub label_side: Vector2,
}

struct Projector<'a, S> {
    screen: &'a S,
    units_per_point: f64,
}

impl<'a, S: Screen> Projector<'a, S> {
    fn new(screen: &'a S, at: Point2) -> Option<Self> {
        let center = screen.to_screen(at)?;
        let across = screen.to_screen(at + Vector2::X)?.distance(center);
        let up = screen.to_screen(at + Vector2::Y)?.distance(center);
        let points_per_unit = (across + up) / 2.0;
        (points_per_unit.is_finite() && points_per_unit > DEGENERATE).then(|| Self {
            screen,
            units_per_point: points_per_unit.recip(),
        })
    }

    fn units(&self, points: f64) -> f64 {
        points * self.units_per_point
    }

    fn point(&self, point: Point2) -> Option<Vector2> {
        self.screen.to_screen(point)
    }

    fn polyline(&self, points: &[Point2]) -> Option<Vec<Vector2>> {
        points.iter().map(|point| self.point(*point)).collect()
    }

    fn direction(&self, at: Point2, direction: Vector2) -> Option<Vector2> {
        let ahead = self.point(at + direction * self.units(1.0))?;
        (ahead - self.point(at)?).try_normalize()
    }

    fn arrow(&self, tip: Point2, direction: Vector2) -> Option<Arrow> {
        let back = tip - direction * self.units(ARROW_LENGTH);
        let tip = self.point(tip)?;
        Some(Arrow {
            tip,
            direction: (tip - self.point(back)?).try_normalize()?,
        })
    }
}

pub fn layout(
    measured: &Measured,
    screen: &impl Screen,
    centre: Option<Point2>,
    lane: usize,
) -> Option<DimensionLayout> {
    let offset = DIMENSION_OFFSET + LANE_SPACING * lane as f64;
    match *measured {
        Measured::Points(a, b) => points_layout(screen, a, b, centre, offset),
        Measured::Aligned { from, to, along } => {
            aligned_layout(screen, from, to, along, centre, offset)
        }
        Measured::PointToLine(point, line) => point_to_line_layout(screen, point, line, centre),
        Measured::PointToCircle {
            point,
            center,
            radius,
        } => point_to_circle_layout(screen, point, center, radius),
        Measured::Angle(first, second, reversed) => angle_layout(screen, first, second, reversed),
        Measured::Radius {
            center,
            radius,
            toward,
        } => radius_layout(screen, center, radius, toward),
        Measured::Diameter {
            center,
            radius,
            toward,
        } => diameter_layout(screen, center, radius, toward),
        Measured::AlongArc { arc, from_centre } => along_arc_layout(screen, &arc, from_centre),
    }
}

fn along_arc_layout(
    screen: &impl Screen,
    arc: &ArcGeometry,
    from_centre: bool,
) -> Option<DimensionLayout> {
    let middle = arc.point_at(arc.start_angle + arc.sweep / 2.0);
    let projector = Projector::new(screen, middle)?;
    let radius = arc.radius + projector.units(DIMENSION_OFFSET);
    let ray = |fraction: f64| Vector2::from_angle(arc.start_angle + arc.sweep * fraction);
    let segments = (arc.sweep / ARC_STEP).ceil().max(MIN_ARC_SEGMENTS);
    let dimension: Vec<Point2> = (0..=segments as usize)
        .map(|index| arc.center + ray(index as f64 / segments) * radius)
        .collect();
    let mut strokes = vec![projector.polyline(&dimension)?];
    for fraction in [0.0, 1.0] {
        let outward = ray(fraction);
        let inner = if from_centre {
            arc.center
        } else {
            arc.center + outward * (arc.radius + projector.units(EXTENSION_GAP))
        };
        let outer = arc.center + outward * (radius + projector.units(EXTENSION_OVERSHOOT));
        strokes.push(projector.polyline(&[inner, outer])?);
    }
    let bisector = ray(0.5);
    let label_at = arc.center + bisector * radius;
    Some(DimensionLayout {
        strokes,
        arrows: vec![
            projector.arrow(arc.center + ray(0.0) * radius, -ray(0.0).perp())?,
            projector.arrow(arc.center + ray(1.0) * radius, ray(1.0).perp())?,
        ],
        label: projector.point(label_at)?,
        label_side: projector.direction(label_at, bisector)?,
    })
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Track {
    along: Vector2,
    side: Vector2,
    baseline: f64,
    span: (f64, f64),
}

impl Track {
    fn of(measured: &Measured, centre: Option<Point2>) -> Option<Self> {
        let (from, to, along) = match *measured {
            Measured::Points(a, b) => (a, b, (b - a).try_normalize()?),
            Measured::Aligned { from, to, along } => (from, to, along.try_normalize()?),
            _ => return None,
        };
        let along = if along.x < -DEGENERATE || (along.x.abs() <= DEGENERATE && along.y < 0.0) {
            -along
        } else {
            along
        };
        let side = away_from(along.perp(), (from + to) / 2.0, centre);
        let baseline = match measured {
            Measured::Aligned { .. } => from.dot(side).max(to.dot(side)),
            _ => from.dot(side),
        };
        let (first, second) = (from.dot(along), to.dot(along));
        Some(Self {
            along,
            side,
            baseline,
            span: (first.min(second), first.max(second)),
        })
    }

    fn shares_line_with(&self, other: &Self, tolerance: f64) -> bool {
        self.along.distance(other.along) <= LANE_DIRECTION_TOLERANCE
            && self.side.dot(other.side) > 0.0
            && (self.baseline - other.baseline).abs() <= tolerance
            && self.span.0 < other.span.1 - tolerance
            && other.span.0 < self.span.1 - tolerance
    }

    fn length(&self) -> f64 {
        self.span.1 - self.span.0
    }
}

pub fn lanes(measured: &[Option<Measured>], centre: Option<Point2>, extent: f64) -> Vec<usize> {
    let tolerance = extent.max(1.0) * LANE_TOLERANCE;
    let tracks: Vec<Option<Track>> = measured
        .iter()
        .map(|measured| {
            measured
                .as_ref()
                .and_then(|measured| Track::of(measured, centre))
        })
        .collect();
    let mut order: Vec<usize> = (0..tracks.len()).collect();
    order.sort_by(|a, b| {
        let length = |index: &usize| {
            tracks
                .get(*index)
                .copied()
                .flatten()
                .map_or(0.0, |track| track.length())
        };
        length(a).total_cmp(&length(b))
    });
    let mut lanes = vec![0; tracks.len()];
    let mut placed: Vec<(Track, usize)> = Vec::new();
    for index in order {
        let Some(track) = tracks.get(index).copied().flatten() else {
            continue;
        };
        let taken: Vec<usize> = placed
            .iter()
            .filter(|(other, _)| track.shares_line_with(other, tolerance))
            .map(|(_, lane)| *lane)
            .collect();
        let lane = (0..).find(|lane| !taken.contains(lane)).unwrap_or(0);
        if let Some(slot) = lanes.get_mut(index) {
            *slot = lane;
        }
        placed.push((track, lane));
    }
    lanes
}

fn away_from(normal: Vector2, at: Point2, centre: Option<Point2>) -> Vector2 {
    let outward = centre.map_or(0.0, |centre| normal.dot(at - centre));
    let preference = if outward.abs() > DEGENERATE {
        outward
    } else if normal.y.abs() > DEGENERATE {
        normal.y
    } else {
        -normal.x
    };
    if preference < 0.0 { -normal } else { normal }
}

fn toward(normal: Vector2, at: Vector2, centre: Option<Vector2>) -> Vector2 {
    let inward = centre.map_or(0.0, |centre| normal.dot(centre - at));
    let preference = if inward.abs() > DEGENERATE {
        inward
    } else if normal.y.abs() > DEGENERATE {
        normal.y
    } else {
        normal.x
    };
    if preference < 0.0 { -normal } else { normal }
}

fn points_layout(
    screen: &impl Screen,
    a: Point2,
    b: Point2,
    centre: Option<Point2>,
    reach: f64,
) -> Option<DimensionLayout> {
    let middle = (a + b) / 2.0;
    let projector = Projector::new(screen, middle)?;
    let along = (b - a).try_normalize().unwrap_or(Vector2::X);
    let normal = away_from(along.perp(), middle, centre);
    let offset = normal * projector.units(reach);
    let extension = |point: Point2| {
        projector.polyline(&[
            point + normal * projector.units(EXTENSION_GAP),
            point + normal * projector.units(reach + EXTENSION_OVERSHOOT),
        ])
    };
    let (start, end) = (a + offset, b + offset);
    Some(DimensionLayout {
        strokes: vec![
            extension(a)?,
            extension(b)?,
            projector.polyline(&[start, end])?,
        ],
        arrows: vec![
            projector.arrow(start, -along)?,
            projector.arrow(end, along)?,
        ],
        label: projector.point(middle + offset)?,
        label_side: Vector2::ZERO,
    })
}

fn aligned_layout(
    screen: &impl Screen,
    from: Point2,
    to: Point2,
    along: Vector2,
    centre: Option<Point2>,
    offset: f64,
) -> Option<DimensionLayout> {
    let middle = (from + to) / 2.0;
    let projector = Projector::new(screen, middle)?;
    let side = away_from(along.perp(), middle, centre);
    let reach = from.dot(side).max(to.dot(side)) + projector.units(offset);
    let foot = |point: Point2| point + side * (reach - point.dot(side));
    let (start, end) = (foot(from), foot(to));
    let extension = |point: Point2, foot: Point2| {
        let length = (foot - point).dot(side);
        let gap = projector.units(EXTENSION_GAP).min(length);
        projector.polyline(&[
            point + side * gap,
            foot + side * projector.units(EXTENSION_OVERSHOOT),
        ])
    };
    let direction = (end - start).try_normalize().unwrap_or(along);
    Some(DimensionLayout {
        strokes: vec![
            extension(from, start)?,
            extension(to, end)?,
            projector.polyline(&[start, end])?,
        ],
        arrows: vec![
            projector.arrow(start, -direction)?,
            projector.arrow(end, direction)?,
        ],
        label: projector.point((start + end) / 2.0)?,
        label_side: Vector2::ZERO,
    })
}

fn point_to_circle_layout(
    screen: &impl Screen,
    point: Point2,
    center: Point2,
    radius: f64,
) -> Option<DimensionLayout> {
    let outward = (point - center).try_normalize().unwrap_or(Vector2::X);
    let on_circle = center + outward * radius;
    let projector = Projector::new(screen, (point + on_circle) / 2.0)?;
    let strokes = vec![projector.polyline(&[on_circle, point])?];
    let Some(along) = (point - on_circle).try_normalize() else {
        return Some(DimensionLayout {
            strokes,
            arrows: Vec::new(),
            label: projector.point(point)?,
            label_side: projector.direction(point, outward)?,
        });
    };
    Some(DimensionLayout {
        strokes,
        arrows: vec![
            projector.arrow(on_circle, -along)?,
            projector.arrow(point, along)?,
        ],
        label: projector.point((point + on_circle) / 2.0)?,
        label_side: Vector2::ZERO,
    })
}

fn point_to_line_layout(
    screen: &impl Screen,
    point: Point2,
    line: LineSpan,
    centre: Option<Point2>,
) -> Option<DimensionLayout> {
    let foot = line.foot(point);
    let projector = Projector::new(screen, (point + foot) / 2.0)?;
    let mut strokes = vec![projector.polyline(&[foot, point])?];
    let parameter = line.parameter(foot);
    let clamped = line.clamped(parameter);
    if (parameter - clamped).abs() > DEGENERATE {
        let beyond = (parameter - clamped).signum() * projector.units(EXTENSION_OVERSHOOT);
        strokes.push(projector.polyline(&[line.at(clamped), line.at(parameter + beyond)])?);
    }
    let Some(along) = (point - foot).try_normalize() else {
        let side = away_from(line.direction.perp(), point, centre);
        return Some(DimensionLayout {
            strokes,
            arrows: Vec::new(),
            label: projector.point(point)?,
            label_side: projector.direction(point, side)?,
        });
    };
    Some(DimensionLayout {
        strokes,
        arrows: vec![
            projector.arrow(foot, -along)?,
            projector.arrow(point, along)?,
        ],
        label: projector.point((point + foot) / 2.0)?,
        label_side: Vector2::ZERO,
    })
}

fn angle_layout(
    screen: &impl Screen,
    first: LineSpan,
    second: LineSpan,
    reversed: bool,
) -> Option<DimensionLayout> {
    let sine = first.direction.perp_dot(second.direction);
    if sine.abs() < PARALLEL_SINE {
        return parallel_layout(screen, first, second);
    }
    let across = (second.origin - first.origin).perp_dot(second.direction) / sine;
    let vertex = first.at(across);
    let first_ray = if reversed {
        -first.direction
    } else {
        first.direction
    };
    let signed = first_ray
        .perp_dot(second.direction)
        .atan2(first_ray.dot(second.direction));
    let toward_segments = [(first, first_ray), (second, second.direction)]
        .iter()
        .filter_map(|(line, ray)| Some((line.middle()? - vertex).dot(*ray)))
        .sum::<f64>();
    let flip = if toward_segments < 0.0 { -1.0 } else { 1.0 };
    let projector = Projector::new(screen, vertex)?;
    let radius = projector.units(ANGLE_RADIUS);
    let start_angle = (first_ray * flip).to_angle();
    let ray = |fraction: f64| Vector2::from_angle(start_angle + signed * fraction);
    let segments = (signed.abs() / ARC_STEP).ceil().max(MIN_ARC_SEGMENTS);
    let arc: Vec<Point2> = (0..=segments as usize)
        .map(|index| vertex + ray(index as f64 / segments) * radius)
        .collect();
    let turning = signed.signum();
    let mut strokes = vec![projector.polyline(&arc)?];
    for (line, fraction) in [(first, 0.0), (second, 1.0)] {
        let end = vertex + ray(fraction) * radius;
        let nearest = line.closest(end);
        let outside = end - nearest;
        if outside.length() > projector.units(MIN_EXTENSION) {
            let beyond = end + outside.normalize() * projector.units(EXTENSION_OVERSHOOT);
            strokes.push(projector.polyline(&[nearest, beyond])?);
        }
    }
    let bisector = ray(0.5);
    let label_at = vertex + bisector * radius;
    Some(DimensionLayout {
        strokes,
        arrows: vec![
            projector.arrow(vertex + ray(0.0) * radius, -ray(0.0).perp() * turning)?,
            projector.arrow(vertex + ray(1.0) * radius, ray(1.0).perp() * turning)?,
        ],
        label: projector.point(label_at)?,
        label_side: projector.direction(label_at, bisector)?,
    })
}

fn parallel_layout(
    screen: &impl Screen,
    first: LineSpan,
    second: LineSpan,
) -> Option<DimensionLayout> {
    let candidates = [(first, second), (second, first)]
        .into_iter()
        .flat_map(|(from, to)| {
            from.ends()
                .into_iter()
                .map(move |end| (end, to.closest(end)))
        });
    let (end, across) =
        candidates.min_by(|a, b| a.0.distance(a.1).total_cmp(&b.0.distance(b.1)))?;
    let middle = (end + across) / 2.0;
    let projector = Projector::new(screen, middle)?;
    Some(DimensionLayout {
        strokes: Vec::new(),
        arrows: Vec::new(),
        label: projector.point(middle)?,
        label_side: Vector2::ZERO,
    })
}

fn radius_layout(
    screen: &impl Screen,
    center: Point2,
    radius: f64,
    toward: Vector2,
) -> Option<DimensionLayout> {
    let on_curve = center + toward * radius;
    let projector = Projector::new(screen, on_curve)?;
    let end = on_curve + toward * projector.units(RADIUS_OVERSHOOT);
    Some(DimensionLayout {
        strokes: vec![projector.polyline(&[center, end])?],
        arrows: vec![projector.arrow(on_curve, toward)?],
        label: projector.point(end)?,
        label_side: projector.direction(end, toward)?,
    })
}

fn diameter_layout(
    screen: &impl Screen,
    center: Point2,
    radius: f64,
    toward: Vector2,
) -> Option<DimensionLayout> {
    let (near, far) = (center - toward * radius, center + toward * radius);
    let projector = Projector::new(screen, far)?;
    let end = far + toward * projector.units(RADIUS_OVERSHOOT);
    Some(DimensionLayout {
        strokes: vec![projector.polyline(&[near, end])?],
        arrows: vec![
            projector.arrow(far, toward)?,
            projector.arrow(near, -toward)?,
        ],
        label: projector.point(end)?,
        label_side: projector.direction(end, toward)?,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum GlyphKind {
    Horizontal,
    Vertical,
    Parallel,
    Perpendicular,
    Tangent,
    Equal,
    Coincident,
    OnCurve,
    Midpoint,
    Concentric,
    Collinear,
    Symmetric,
    Fix,
}

pub fn glyphs_of(sketch: &Sketch, constraint: &Constraint) -> Vec<(EntityId, GlyphKind)> {
    let on_each = |kind: GlyphKind, entities: [EntityId; 2]| {
        entities
            .into_iter()
            .filter(|entity| !entity.is_reference())
            .map(|entity| (entity, kind))
            .collect()
    };
    match *constraint {
        Constraint::Horizontal(line) => vec![(line, GlyphKind::Horizontal)],
        Constraint::Vertical(line) => vec![(line, GlyphKind::Vertical)],
        Constraint::HorizontalPoints(a, b) => on_each(GlyphKind::Horizontal, [a, b]),
        Constraint::VerticalPoints(a, b) => on_each(GlyphKind::Vertical, [a, b]),
        Constraint::Midpoint { point, .. } => vec![(point, GlyphKind::Midpoint)],
        Constraint::Concentric(a, b) => on_each(GlyphKind::Concentric, [a, b]),
        Constraint::Collinear(a, b) => on_each(GlyphKind::Collinear, [a, b]),
        Constraint::Symmetric { first, second, .. } => {
            on_each(GlyphKind::Symmetric, [first, second])
        }
        Constraint::Fix { point, .. } => vec![(point, GlyphKind::Fix)],
        Constraint::Parallel(a, b) => on_each(GlyphKind::Parallel, [a, b]),
        Constraint::Perpendicular(a, b) => on_each(GlyphKind::Perpendicular, [a, b]),
        Constraint::Tangent(a, b) => on_each(GlyphKind::Tangent, [a, b]),
        Constraint::Equal(a, b) => on_each(GlyphKind::Equal, [a, b]),
        Constraint::Coincident(a, b) => {
            let is_point = |entity| sketch.point(entity).is_some();
            let anchor = match (is_point(a), is_point(b)) {
                (true, true) => [a, b]
                    .into_iter()
                    .find(|entity| !entity.is_reference())
                    .map(|entity| (entity, GlyphKind::Coincident)),
                (true, false) => Some((a, GlyphKind::OnCurve)),
                (false, true) => Some((b, GlyphKind::OnCurve)),
                (false, false) => None,
            };
            anchor.into_iter().collect()
        }
        Constraint::Distance { .. }
        | Constraint::HorizontalDistance { .. }
        | Constraint::VerticalDistance { .. }
        | Constraint::Angle { .. }
        | Constraint::Radius { .. }
        | Constraint::Diameter { .. }
        | Constraint::ArcLength { .. }
        | Constraint::Sweep { .. } => Vec::new(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GlyphAnchor {
    Segment(Vector2, Vector2),
    Point(Vector2),
    Curve { at: Vector2, outward: Vector2 },
}

pub fn glyph_anchor(
    sketch: &Sketch,
    entity: EntityId,
    screen: &impl Screen,
) -> Option<GlyphAnchor> {
    let on_curve = |center: Point2, at: Point2| {
        let center = screen.to_screen(center)?;
        let at = screen.to_screen(at)?;
        Some(GlyphAnchor::Curve {
            at,
            outward: (at - center).try_normalize()?,
        })
    };
    if entity == EntityId::ORIGIN {
        return screen.to_screen(Point2::ZERO).map(GlyphAnchor::Point);
    }
    match sketch.entity(entity)? {
        Entity::Point(position) => screen.to_screen(*position).map(GlyphAnchor::Point),
        Entity::Line { .. } => {
            let (start, end) = sketch.line_endpoints(entity)?;
            Some(GlyphAnchor::Segment(
                screen.to_screen(start)?,
                screen.to_screen(end)?,
            ))
        }
        Entity::Circle { .. } => {
            let (center, radius) = sketch.circle(entity)?;
            on_curve(center, center + CIRCLE_GLYPH_DIRECTION * radius)
        }
        Entity::Arc { .. } => {
            let arc = sketch.arc(entity)?;
            on_curve(
                arc.center,
                arc.point_at(arc.start_angle + arc.sweep * ARC_GLYPH_FRACTION),
            )
        }
        Entity::Spline { .. } => {
            let points = sketch.polyline(entity, SPLINE_GLYPH_ANGLE)?;
            let middle = points.len() / 2;
            let (before, at) = (*points.get(middle.checked_sub(1)?)?, *points.get(middle)?);
            let at = screen.to_screen(at)?;
            let before = screen.to_screen(before)?;
            Some(GlyphAnchor::Curve {
                at,
                outward: (at - before).try_normalize()?.perp(),
            })
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Footprint {
    pub center: Vector2,
    pub half: Vector2,
}

impl Footprint {
    pub fn overlap(&self, other: &Self) -> f64 {
        let reach = self.half + other.half;
        let apart = (self.center - other.center).abs();
        let narrower = self.half.min(other.half) * 2.0;
        let shared = (reach - apart).min(narrower).max(Vector2::ZERO);
        shared.x * shared.y
    }
}

#[derive(Debug, Clone, Default)]
pub struct Obstacles {
    footprints: Vec<Footprint>,
    cells: BTreeMap<(i64, i64), Vec<usize>>,
}

impl Obstacles {
    pub fn add(&mut self, footprint: Footprint) {
        let index = self.footprints.len();
        self.footprints.push(footprint);
        for cell in cells_under(&footprint) {
            self.cells.entry(cell).or_default().push(index);
        }
    }

    fn overlap(&self, footprint: &Footprint) -> f64 {
        let mut near: Vec<usize> = cells_under(footprint)
            .filter_map(|cell| self.cells.get(&cell))
            .flatten()
            .copied()
            .collect();
        near.sort_unstable();
        near.dedup();
        near.into_iter()
            .filter_map(|index| self.footprints.get(index))
            .map(|other| footprint.overlap(other))
            .sum()
    }
}

fn cells_under(footprint: &Footprint) -> impl Iterator<Item = (i64, i64)> {
    let cell = |at: f64| (at / OBSTACLE_CELL).floor() as i64;
    let low = footprint.center - footprint.half;
    let high = footprint.center + footprint.half;
    let span = |low: f64, high: f64| {
        let first = cell(low);
        first..=cell(high).min(first.saturating_add(MAX_OBSTACLE_CELLS))
    };
    let (columns, rows) = (span(low.x, high.x), span(low.y, high.y));
    columns.flat_map(move |column| rows.clone().map(move |row| (column, row)))
}

pub fn place_glyphs(
    anchor: GlyphAnchor,
    count: usize,
    centre: Option<Vector2>,
    half: Vector2,
    blocked: &Obstacles,
) -> Vec<Vector2> {
    let overlap = |positions: &[Vector2]| {
        positions
            .iter()
            .map(|center| {
                blocked.overlap(&Footprint {
                    center: *center,
                    half,
                })
            })
            .sum::<f64>()
    };
    let mut best: Option<(f64, Vec<Vector2>)> = None;
    for placement in placements(anchor, count) {
        let positions = stacked(anchor, count, centre, placement);
        let overlapping = overlap(&positions);
        if overlapping <= 0.0 {
            return positions;
        }
        if best.as_ref().is_none_or(|(least, _)| overlapping < *least) {
            best = Some((overlapping, positions));
        }
    }
    best.map(|(_, positions)| positions).unwrap_or_default()
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Placement {
    flipped: bool,
    shift: f64,
    quadrant: usize,
}

impl Default for Placement {
    fn default() -> Self {
        Self {
            flipped: false,
            shift: 0.0,
            quadrant: 0,
        }
    }
}

fn placements(anchor: GlyphAnchor, count: usize) -> impl Iterator<Item = Placement> {
    let extent = count.saturating_sub(1) as f64 * GLYPH_SPACING;
    let reach = match anchor {
        GlyphAnchor::Segment(start, end) => Some(((start.distance(end) - extent) / 2.0).max(0.0)),
        GlyphAnchor::Curve { .. } => Some(MAX_CURVE_GLYPH_SHIFT),
        GlyphAnchor::Point(_) => None,
    };
    let quadrants = reach
        .is_none()
        .then_some(0..POINT_GLYPH_QUADRANTS.len())
        .into_iter()
        .flatten()
        .map(|quadrant| Placement {
            quadrant,
            ..Placement::default()
        });
    let shifted = reach.into_iter().flat_map(|reach| {
        let within = ((reach / GLYPH_SHIFT_STEP).floor() as usize).min(MAX_GLYPH_SHIFTS);
        let shifts = |first: usize, last: usize| {
            (first..=last).flat_map(|step| {
                let shift = step as f64 * GLYPH_SHIFT_STEP;
                [shift, -shift]
            })
        };
        let sides = move |centre: bool, first: usize, last: usize| {
            [false, true].into_iter().flat_map(move |flipped| {
                centre
                    .then_some(0.0)
                    .into_iter()
                    .chain(shifts(first, last))
                    .map(move |shift| Placement {
                        flipped,
                        shift,
                        quadrant: 0,
                    })
            })
        };
        sides(true, 1, within).chain(sides(false, within + 1, within + GLYPH_STEPS_BEYOND))
    });
    quadrants.chain(shifted)
}

pub fn within_view(anchor: GlyphAnchor, size: Vector2) -> Option<GlyphAnchor> {
    let low = Vector2::splat(-GLYPH_VIEW_MARGIN);
    let high = size + Vector2::splat(GLYPH_VIEW_MARGIN);
    let inside = |point: Vector2| point.cmpge(low).all() && point.cmple(high).all();
    match anchor {
        GlyphAnchor::Point(at) | GlyphAnchor::Curve { at, .. } => inside(at).then_some(anchor),
        GlyphAnchor::Segment(start, end) => {
            let along = end - start;
            let mut enter: f64 = 0.0;
            let mut leave: f64 = 1.0;
            for axis in 0..2 {
                let (from, step) = (start[axis], along[axis]);
                if step.abs() <= DEGENERATE {
                    if from < low[axis] || from > high[axis] {
                        return None;
                    }
                    continue;
                }
                let first = (low[axis] - from) / step;
                let second = (high[axis] - from) / step;
                enter = enter.max(first.min(second));
                leave = leave.min(first.max(second));
            }
            (enter <= leave)
                .then(|| GlyphAnchor::Segment(start + along * enter, start + along * leave))
        }
    }
}

fn stacked(
    anchor: GlyphAnchor,
    count: usize,
    centre: Option<Vector2>,
    placement: Placement,
) -> Vec<Vector2> {
    let side = if placement.flipped { -1.0 } else { 1.0 };
    let centred = |base: Vector2, step: Vector2| {
        let middle = count.saturating_sub(1) as f64 / 2.0;
        let along = step.try_normalize().unwrap_or(Vector2::ZERO);
        (0..count)
            .map(|index| base + along * placement.shift + step * (index as f64 - middle))
            .collect()
    };
    match anchor {
        GlyphAnchor::Segment(start, end) => {
            let middle = (start + end) / 2.0;
            let along = (end - start).try_normalize().unwrap_or(Vector2::X);
            let normal = toward(along.perp(), middle, centre) * side;
            centred(middle + normal * GLYPH_OFFSET, along * GLYPH_SPACING)
        }
        GlyphAnchor::Curve { at, outward } => centred(
            at + outward * side * GLYPH_OFFSET,
            outward.perp() * GLYPH_SPACING,
        ),
        GlyphAnchor::Point(at) => {
            let quadrant = POINT_GLYPH_QUADRANTS
                .get(placement.quadrant)
                .copied()
                .unwrap_or(Vector2::ONE);
            let offset = POINT_GLYPH_OFFSET * quadrant;
            (0..count)
                .map(|index| {
                    at + offset + Vector2::X * quadrant.x * POINT_GLYPH_SPACING * index as f64
                })
                .collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use caditor_geometry::Plane;

    use super::*;

    struct Flat;

    impl Screen for Flat {
        fn to_screen(&self, point: Point2) -> Option<Vector2> {
            Some(Vector2::new(100.0 + 2.0 * point.x, 300.0 - 2.0 * point.y))
        }
    }

    fn close(a: Vector2, b: Vector2) -> bool {
        a.distance(b) < 1e-9
    }

    fn assert_close(actual: Vector2, expected: Vector2) {
        assert!(close(actual, expected), "{actual} is not {expected}");
    }

    const GLYPH_HALF: Vector2 = Vector2::splat(8.0);

    fn stack_glyphs(anchor: GlyphAnchor, count: usize, centre: Option<Vector2>) -> Vec<Vector2> {
        place_glyphs(anchor, count, centre, GLYPH_HALF, &Obstacles::default())
    }

    fn label_at(center: Vector2, size: Vector2) -> Obstacles {
        let mut obstacles = Obstacles::default();
        obstacles.add(Footprint {
            center,
            half: size / 2.0,
        });
        obstacles
    }

    #[test]
    fn a_horizontal_distance_sits_above_the_line_away_from_the_sketch() {
        let measured = Measured::Points(Point2::ZERO, Point2::new(40.0, 0.0));
        let layout = layout(&measured, &Flat, Some(Point2::new(20.0, -10.0)), 0).unwrap();

        assert_close(layout.label, Vector2::new(140.0, 272.0));
        assert_eq!(layout.label_side, Vector2::ZERO);
        assert_close(layout.arrows[0].tip, Vector2::new(100.0, 272.0));
        assert_close(layout.arrows[0].direction, Vector2::new(-1.0, 0.0));
        assert_close(layout.arrows[1].tip, Vector2::new(180.0, 272.0));
        assert_close(layout.arrows[1].direction, Vector2::new(1.0, 0.0));
        assert_close(layout.strokes[0][0], Vector2::new(100.0, 296.0));
        assert_close(layout.strokes[0][1], Vector2::new(100.0, 267.0));
        assert_eq!(
            layout.strokes[2],
            vec![Vector2::new(100.0, 272.0), Vector2::new(180.0, 272.0)]
        );

        let below = self::layout(&measured, &Flat, Some(Point2::new(20.0, 10.0)), 0).unwrap();
        assert_close(below.label, Vector2::new(140.0, 328.0));
    }

    #[test]
    fn a_rotated_distance_is_parallel_to_the_measured_segment() {
        let measured = Measured::Points(Point2::ZERO, Point2::new(30.0, 40.0));
        let layout = layout(&measured, &Flat, None, 0).unwrap();

        assert_close(layout.arrows[0].tip, Vector2::new(77.6, 283.2));
        assert_close(layout.arrows[0].direction, Vector2::new(-0.6, 0.8));
        assert_close(layout.arrows[1].tip, Vector2::new(137.6, 203.2));
        assert_close(layout.arrows[1].direction, Vector2::new(0.6, -0.8));
        assert_close(layout.label, Vector2::new(107.6, 243.2));
    }

    #[test]
    fn a_point_to_line_distance_is_perpendicular_and_extends_the_line() {
        let mut sketch = Sketch::new(Plane::XY);
        let line = sketch.add_line(Point2::ZERO, Point2::new(10.0, 0.0));
        let point = sketch.add_point(Point2::new(20.0, 5.0));
        let measured = measured(
            &sketch,
            &Constraint::Distance {
                from: point,
                to: line,
                value: caditor_expression::Expression::Number(5.0),
            },
        )
        .unwrap();
        let layout = layout(&measured, &Flat, None, 0).unwrap();

        assert_close(layout.arrows[0].tip, Vector2::new(140.0, 300.0));
        assert_close(layout.arrows[0].direction, Vector2::new(0.0, 1.0));
        assert_close(layout.arrows[1].tip, Vector2::new(140.0, 290.0));
        assert_close(layout.label, Vector2::new(140.0, 295.0));
        assert_eq!(
            layout.strokes[1],
            vec![Vector2::new(120.0, 300.0), Vector2::new(145.0, 300.0)]
        );
    }

    #[test]
    fn an_angle_is_an_arc_from_the_first_line_to_the_second() {
        let mut sketch = Sketch::new(Plane::XY);
        let first = sketch.add_line(Point2::ZERO, Point2::new(40.0, 0.0));
        let second = sketch.add_line(Point2::ZERO, Point2::new(20.0, 20.0));
        let measured = measured(
            &sketch,
            &Constraint::Angle {
                from: first,
                to: second,
                reversed: false,
                value: caditor_expression::Expression::Number(45.0),
            },
        )
        .unwrap();
        let layout = layout(&measured, &Flat, None, 0).unwrap();

        let half = PI / 8.0;
        assert_close(
            layout.label,
            Vector2::new(100.0 + 44.0 * half.cos(), 300.0 - 44.0 * half.sin()),
        );
        assert_close(layout.label_side, Vector2::new(half.cos(), -half.sin()));
        assert_close(layout.arrows[0].tip, Vector2::new(144.0, 300.0));
        assert_close(layout.arrows[0].direction, Vector2::new(0.0, 1.0));
        let end = Vector2::new(FRAC_1_SQRT_2, -FRAC_1_SQRT_2);
        assert_close(
            layout.arrows[1].tip,
            Vector2::new(100.0, 300.0) + end * 44.0,
        );
        assert_close(
            layout.arrows[1].direction,
            Vector2::new(-FRAC_1_SQRT_2, -FRAC_1_SQRT_2),
        );
        assert_eq!(layout.strokes.len(), 1);
        let arc = &layout.strokes[0];
        assert!(
            arc.iter()
                .all(|point| (point.distance(Vector2::new(100.0, 300.0)) - 44.0).abs() < 1e-9)
        );
    }

    #[test]
    fn nearly_parallel_lines_label_the_angle_between_their_closest_ends() {
        let mut sketch = Sketch::new(Plane::XY);
        let first = sketch.add_line(Point2::ZERO, Point2::new(40.0, 0.0));
        let second = sketch.add_line(Point2::new(50.0, 10.0), Point2::new(90.0, 10.0));
        let measured = measured(
            &sketch,
            &Constraint::Angle {
                from: first,
                to: second,
                reversed: false,
                value: caditor_expression::Expression::Number(0.0),
            },
        )
        .unwrap();
        let layout = layout(&measured, &Flat, None, 0).unwrap();

        assert!(layout.arrows.is_empty());
        assert_close(
            layout.label,
            Flat.to_screen(Point2::new(45.0, 5.0)).unwrap(),
        );
    }

    #[test]
    fn a_radius_is_a_leader_from_the_centre_through_the_curve() {
        let mut sketch = Sketch::new(Plane::XY);
        let circle = sketch.add_circle(Point2::new(10.0, 10.0), 5.0);
        let measured = measured(
            &sketch,
            &Constraint::Radius {
                entity: circle,
                value: caditor_expression::Expression::Number(5.0),
            },
        )
        .unwrap();
        let layout = layout(&measured, &Flat, None, 0).unwrap();

        let outward = Vector2::new(FRAC_1_SQRT_2, -FRAC_1_SQRT_2);
        let on_curve = Vector2::new(120.0, 280.0) + outward * 10.0;
        assert_close(layout.arrows[0].tip, on_curve);
        assert_close(layout.arrows[0].direction, outward);
        assert_close(layout.strokes[0][0], Vector2::new(120.0, 280.0));
        assert_close(layout.label, on_curve + outward * 24.0);
        assert_close(layout.label_side, outward);
    }

    #[test]
    fn a_horizontal_distance_is_measured_level_above_the_higher_point() {
        let mut sketch = Sketch::new(Plane::XY);
        let from = sketch.add_point(Point2::ZERO);
        let to = sketch.add_point(Point2::new(30.0, 10.0));
        let measured = measured(
            &sketch,
            &Constraint::HorizontalDistance {
                from,
                to,
                value: caditor_expression::Expression::Number(30.0),
            },
        )
        .unwrap();
        let layout = layout(&measured, &Flat, Some(Point2::new(15.0, -20.0)), 0).unwrap();

        assert_close(layout.arrows[0].tip, Vector2::new(100.0, 252.0));
        assert_close(layout.arrows[0].direction, Vector2::new(-1.0, 0.0));
        assert_close(layout.arrows[1].tip, Vector2::new(160.0, 252.0));
        assert_close(layout.label, Vector2::new(130.0, 252.0));
        assert_eq!(
            layout.strokes[0],
            vec![Vector2::new(100.0, 296.0), Vector2::new(100.0, 247.0)]
        );
        assert_eq!(
            layout.strokes[1],
            vec![Vector2::new(160.0, 276.0), Vector2::new(160.0, 247.0)]
        );
    }

    #[test]
    fn a_diameter_spans_the_circle_and_a_point_to_circle_distance_meets_it_squarely() {
        let mut sketch = Sketch::new(Plane::XY);
        let circle = sketch.add_circle(Point2::new(10.0, 10.0), 5.0);
        let point = sketch.add_point(Point2::new(25.0, 10.0));
        let diameter = measured(
            &sketch,
            &Constraint::Diameter {
                entity: circle,
                value: caditor_expression::Expression::Number(10.0),
            },
        )
        .unwrap();
        let spanned = layout(&diameter, &Flat, None, 0).unwrap();

        let outward = Vector2::new(FRAC_1_SQRT_2, -FRAC_1_SQRT_2);
        let centre = Vector2::new(120.0, 280.0);
        assert_close(spanned.arrows[0].tip, centre + outward * 10.0);
        assert_close(spanned.arrows[1].tip, centre - outward * 10.0);
        assert_close(spanned.arrows[1].direction, -outward);
        assert_close(spanned.strokes[0][0], centre - outward * 10.0);
        assert_close(spanned.label, centre + outward * 34.0);

        let distance = measured(
            &sketch,
            &Constraint::Distance {
                from: circle,
                to: point,
                value: caditor_expression::Expression::Number(10.0),
            },
        )
        .unwrap();
        let squarely = layout(&distance, &Flat, None, 0).unwrap();
        assert_close(squarely.arrows[0].tip, Vector2::new(130.0, 280.0));
        assert_close(squarely.arrows[1].tip, Vector2::new(150.0, 280.0));
        assert_close(squarely.label, Vector2::new(140.0, 280.0));
    }

    #[test]
    fn an_arc_length_runs_outside_the_arc_and_a_sweep_reaches_back_to_its_centre() {
        let mut sketch = Sketch::new(Plane::XY);
        let arc = sketch.add_arc(Point2::ZERO, Point2::new(10.0, 0.0), Point2::new(0.0, 10.0));
        let value = caditor_expression::Expression::Number(1.0);
        let length = measured(
            &sketch,
            &Constraint::ArcLength {
                arc,
                value: value.clone(),
            },
        )
        .unwrap();
        let sweep = measured(&sketch, &Constraint::Sweep { arc, value }).unwrap();

        let along = layout(&length, &Flat, None, 0).unwrap();
        let centre = Vector2::new(100.0, 300.0);
        let reach = 20.0 + DIMENSION_OFFSET;
        let half = FRAC_1_SQRT_2;
        assert_close(along.label, centre + Vector2::new(half, -half) * reach);
        assert_close(along.arrows[0].tip, centre + Vector2::new(reach, 0.0));
        assert_close(along.arrows[0].direction, Vector2::new(0.0, 1.0));
        assert_close(along.arrows[1].tip, centre + Vector2::new(0.0, -reach));
        assert_close(along.arrows[1].direction, Vector2::new(-1.0, 0.0));
        assert_close(
            along.strokes[1][0],
            Vector2::new(120.0 + EXTENSION_GAP, 300.0),
        );

        let turned = layout(&sweep, &Flat, None, 0).unwrap();
        assert_close(turned.strokes[1][0], centre);
        assert_close(turned.strokes[2][0], centre);
    }

    #[test]
    fn an_overall_dimension_over_a_chain_moves_out_a_lane_and_the_chain_shares_one() {
        let level = |from: f64, to: f64| Measured::Aligned {
            from: Point2::new(from, 0.0),
            to: Point2::new(to, 0.0),
            along: Vector2::X,
        };
        let measured = vec![
            Some(level(0.0, 40.0)),
            Some(level(0.0, 15.0)),
            Some(level(15.0, 40.0)),
            Some(Measured::Points(
                Point2::new(0.0, 30.0),
                Point2::new(0.0, 60.0),
            )),
            None,
        ];
        let centre = Some(Point2::new(20.0, 20.0));

        let lanes = lanes(&measured, centre, 60.0);

        assert_eq!(lanes, vec![1, 0, 0, 0, 0]);
        let near = layout(measured[1].as_ref().unwrap(), &Flat, centre, 0).unwrap();
        let far = layout(measured[0].as_ref().unwrap(), &Flat, centre, 1).unwrap();
        assert_close(far.label - near.label, Vector2::new(25.0, LANE_SPACING));
    }

    #[test]
    fn gaps_between_circles_and_from_a_line_run_between_their_nearest_points() {
        let mut sketch = Sketch::new(Plane::XY);
        let large = sketch.add_circle(Point2::ZERO, 10.0);
        let apart = sketch.add_circle(Point2::new(20.0, 0.0), 4.0);
        let within = sketch.add_circle(Point2::new(0.0, 5.0), 2.0);
        let line = sketch.add_line(Point2::new(-5.0, 30.0), Point2::new(5.0, 30.0));
        let gap = |from, to| {
            measured(
                &sketch,
                &Constraint::Distance {
                    from,
                    to,
                    value: caditor_expression::Expression::Number(1.0),
                },
            )
            .unwrap()
        };

        assert_eq!(
            gap(apart, large),
            Measured::PointToCircle {
                point: Point2::new(16.0, 0.0),
                center: Point2::ZERO,
                radius: 10.0,
            }
        );
        assert_eq!(
            gap(large, within),
            Measured::PointToCircle {
                point: Point2::new(0.0, 7.0),
                center: Point2::ZERO,
                radius: 10.0,
            }
        );
        let Measured::PointToLine(point, span) = gap(line, large) else {
            panic!("expected a gap to the line");
        };
        assert_close(point, Point2::new(0.0, 10.0));
        assert_close(span.foot(point), Point2::new(0.0, 30.0));
    }

    #[test]
    fn glyphs_stack_along_a_line_on_the_side_of_the_sketch() {
        let anchor = GlyphAnchor::Segment(Vector2::ZERO, Vector2::new(100.0, 0.0));
        let below = stack_glyphs(anchor, 2, Some(Vector2::new(50.0, 50.0)));
        assert_eq!(
            below,
            vec![Vector2::new(41.0, 14.0), Vector2::new(59.0, 14.0)]
        );
        let above = stack_glyphs(anchor, 1, Some(Vector2::new(50.0, -50.0)));
        assert_eq!(above, vec![Vector2::new(50.0, -14.0)]);
        let alone = stack_glyphs(anchor, 1, None);
        assert_eq!(alone, vec![Vector2::new(50.0, 14.0)]);
        let upright = stack_glyphs(
            GlyphAnchor::Segment(Vector2::ZERO, Vector2::new(0.0, 100.0)),
            1,
            Some(Vector2::new(0.0, 50.0)),
        );
        assert_eq!(upright, vec![Vector2::new(14.0, 50.0)]);

        let point = stack_glyphs(GlyphAnchor::Point(Vector2::new(5.0, 5.0)), 2, None);
        assert_eq!(
            point,
            vec![Vector2::new(15.0, -5.0), Vector2::new(27.0, -5.0)]
        );
    }

    #[test]
    fn glyphs_move_along_their_line_off_a_label_then_to_its_other_side() {
        let anchor = GlyphAnchor::Segment(Vector2::ZERO, Vector2::new(0.0, 100.0));
        let centre = Some(Vector2::new(50.0, 50.0));
        let label = label_at(Vector2::new(10.0, 50.0), Vector2::new(80.0, 18.0));

        let moved = place_glyphs(anchor, 1, centre, GLYPH_HALF, &label);
        assert_eq!(moved, vec![Vector2::new(14.0, 68.0)]);

        let tall = label_at(Vector2::new(14.0, 50.0), Vector2::new(20.0, 120.0));
        let flipped = place_glyphs(anchor, 1, centre, GLYPH_HALF, &tall);
        assert_eq!(flipped, vec![Vector2::new(-14.0, 50.0)]);

        let everywhere = label_at(Vector2::new(0.0, 50.0), Vector2::new(200.0, 200.0));
        let least = place_glyphs(anchor, 1, centre, GLYPH_HALF, &everywhere);
        assert_eq!(least, vec![Vector2::new(14.0, 50.0)]);

        let point = GlyphAnchor::Point(Vector2::new(5.0, 5.0));
        let upper_right = label_at(Vector2::new(15.0, -5.0), Vector2::new(10.0, 10.0));
        let left = place_glyphs(point, 1, None, GLYPH_HALF, &upper_right);
        assert_eq!(left, vec![Vector2::new(-5.0, -5.0)]);
    }

    #[test]
    fn coincident_points_get_a_dot_and_points_on_curves_a_ring() {
        let mut sketch = Sketch::new(Plane::XY);
        let line = sketch.add_line(Point2::ZERO, Point2::new(10.0, 0.0));
        let lone = sketch.add_point(Point2::new(3.0, 0.0));
        assert_eq!(
            glyphs_of(&sketch, &Constraint::Coincident(EntityId::ORIGIN, lone)),
            vec![(lone, GlyphKind::Coincident)]
        );
        assert_eq!(
            glyphs_of(&sketch, &Constraint::Coincident(line, lone)),
            vec![(lone, GlyphKind::OnCurve)]
        );
        assert_eq!(
            glyphs_of(
                &sketch,
                &Constraint::Parallel(line, EntityId::HORIZONTAL_AXIS)
            ),
            vec![(line, GlyphKind::Parallel)]
        );
        assert_eq!(
            glyphs_of(
                &sketch,
                &Constraint::HorizontalPoints(lone, EntityId::ORIGIN)
            ),
            vec![(lone, GlyphKind::Horizontal)]
        );
        assert_eq!(
            glyphs_of(
                &sketch,
                &Constraint::Midpoint {
                    point: lone,
                    curve: line
                }
            ),
            vec![(lone, GlyphKind::Midpoint)]
        );
        assert_eq!(
            glyphs_of(
                &sketch,
                &Constraint::Fix {
                    point: lone,
                    at: Point2::new(3.0, 0.0)
                }
            ),
            vec![(lone, GlyphKind::Fix)]
        );
    }

    #[test]
    fn a_line_far_longer_than_the_view_offers_few_placements_from_its_visible_middle() {
        let huge = GlyphAnchor::Segment(Vector2::new(-1e7, 50.0), Vector2::new(1e7, 50.0));
        let size = Vector2::new(800.0, 600.0);

        let visible = within_view(huge, size);
        let offered = placements(huge, 1).count();

        let margin = GLYPH_VIEW_MARGIN;
        assert_eq!(
            visible,
            Some(GlyphAnchor::Segment(
                Vector2::new(-margin, 50.0),
                Vector2::new(800.0 + margin, 50.0)
            ))
        );
        assert_eq!(
            offered,
            2 * (1 + 2 * (MAX_GLYPH_SHIFTS + GLYPH_STEPS_BEYOND))
        );
        assert_eq!(placements(huge, 1).next(), Some(Placement::default()));
        let outside =
            GlyphAnchor::Segment(Vector2::new(-500.0, -500.0), Vector2::new(-400.0, -100.0));
        assert_eq!(within_view(outside, size), None);
        let off_screen_point = GlyphAnchor::Point(Vector2::new(2000.0, 10.0));
        assert_eq!(within_view(off_screen_point, size), None);
    }
}
