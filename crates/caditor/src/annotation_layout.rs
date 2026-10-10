use std::f64::consts::{FRAC_1_SQRT_2, PI, TAU};

use caditor_geometry::{Point2, Vector2};
use caditor_sketch::{
    ArcGeometry, Constraint, Entity, EntityId, Sketch,
    annotation::{Footprint, LineSpan, Measured, Obstacles, away_from},
};

use crate::snap::Screen;

const DIMENSION_OFFSET: f64 = 28.0;
const LANE_SPACING: f64 = 22.0;
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
const CIRCLE_GLYPH_DIRECTION: Vector2 = Vector2::new(-FRAC_1_SQRT_2, -FRAC_1_SQRT_2);
const ARC_GLYPH_FRACTION: f64 = 0.25;
const SPLINE_GLYPH_ANGLE: f64 = PI / 18.0;
const GLYPH_SHIFT_STEP: f64 = GLYPH_SPACING / 2.0;
const MAX_CURVE_GLYPH_SHIFT: f64 = GLYPH_SPACING * 2.0;
const GLYPH_STEPS_BEYOND: usize = 4;
const MAX_GLYPH_SHIFTS: usize = 64;
const GLYPH_VIEW_MARGIN: f64 = GLYPH_OFFSET + GLYPH_SPACING;
const OBSTACLE_CELL: f64 = 32.0;
const LABEL_REACH: f64 = 200.0;
const FULL_CELL_SHARE: f64 = 0.5;
const MOST_GLYPH_COVER: f64 = 0.25;
const MOST_LABEL_COVER: f64 = 0.5;
const POINT_GLYPH_QUADRANTS: [Vector2; 4] = [
    Vector2::ONE,
    Vector2::new(-1.0, 1.0),
    Vector2::new(1.0, -1.0),
    Vector2::NEG_ONE,
];

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
    pub at: Point2,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LabelFrame {
    origin: Point2,
    along: Vector2,
}

impl LabelFrame {
    pub fn place(&self, offset: Vector2) -> Point2 {
        self.origin + self.along * offset.x + self.along.perp() * offset.y
    }

    pub fn offset_of(&self, at: Point2) -> Vector2 {
        let from = at - self.origin;
        Vector2::new(from.dot(self.along), from.dot(self.along.perp()))
    }
}

pub fn label_frame(measured: &Measured) -> Option<LabelFrame> {
    let (origin, along) = match *measured {
        Measured::Points(a, b) => ((a + b) / 2.0, (b - a).try_normalize().unwrap_or(Vector2::X)),
        Measured::Aligned { from, to, along } => ((from + to) / 2.0, along.try_normalize()?),
        Measured::PointToLine(point, line) => ((point + line.foot(point)) / 2.0, line.direction()),
        Measured::PointToCircle { point, center, .. } => (
            point,
            (point - center).try_normalize().unwrap_or(Vector2::X),
        ),
        Measured::Angle(first, second, reversed) => match Corner::of(first, second, reversed) {
            Some(corner) => (corner.vertex, corner.start),
            None => (parallel_middle(first, second)?, first.direction()),
        },
        Measured::Radius { center, .. } | Measured::Diameter { center, .. } => (center, Vector2::X),
        Measured::AlongArc { arc, .. } => (arc.center, Vector2::from_angle(arc.start_angle)),
    };
    Some(LabelFrame { origin, along })
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reach {
    low: Point2,
    high: Point2,
}

impl Reach {
    pub fn of(measured: &Measured, frame: Option<LabelFrame>) -> Option<Self> {
        let around = |center: Point2, radius: f64| {
            let corner = Vector2::splat(radius.abs());
            vec![center - corner, center + corner]
        };
        let mut points = match *measured {
            Measured::Points(a, b) => vec![a, b],
            Measured::Aligned { from, to, .. } => vec![from, to],
            Measured::PointToLine(point, line) => {
                let mut points = line.ends();
                points.extend([point, line.foot(point)]);
                points
            }
            Measured::PointToCircle {
                point,
                center,
                radius,
            } => {
                let mut points = around(center, radius);
                points.push(point);
                points
            }
            Measured::Angle(first, second, _) => {
                first.ends().into_iter().chain(second.ends()).collect()
            }
            Measured::Radius { center, radius, .. } | Measured::Diameter { center, radius, .. } => {
                around(center, radius)
            }
            Measured::AlongArc { arc, .. } => around(arc.center, arc.radius),
        };
        points.extend(frame.map(|frame| frame.origin));
        let first = *points.first()?;
        let start = Self {
            low: first,
            high: first,
        };
        Some(points.into_iter().fold(start, Self::including))
    }

    fn including(self, point: Point2) -> Self {
        Self {
            low: self.low.min(point),
            high: self.high.max(point),
        }
    }

    pub fn near_view(
        &self,
        placed: Option<(LabelFrame, Point2)>,
        lane: usize,
        screen: &impl Screen,
        view: Vector2,
    ) -> bool {
        let reach = placed.map_or(*self, |(frame, at)| {
            let swing = Vector2::splat(frame.origin.distance(at));
            self.including(frame.origin - swing)
                .including(frame.origin + swing)
        });
        let corners = [
            reach.low,
            Point2::new(reach.high.x, reach.low.y),
            reach.high,
            Point2::new(reach.low.x, reach.high.y),
        ];
        let mut low = Vector2::splat(f64::INFINITY);
        let mut high = Vector2::splat(f64::NEG_INFINITY);
        for corner in corners {
            let Some(at) = screen.to_screen(corner) else {
                return true;
            };
            low = low.min(at);
            high = high.max(at);
        }
        let margin = Vector2::splat(
            DIMENSION_OFFSET + LANE_SPACING * lane as f64 + ANGLE_RADIUS + LABEL_REACH,
        );
        low.cmple(view + margin).all() && high.cmpge(-margin).all()
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Corner {
    vertex: Point2,
    start: Vector2,
    signed: f64,
}

impl Corner {
    fn of(first: LineSpan, second: LineSpan, reversed: bool) -> Option<Self> {
        let sine = first.direction().perp_dot(second.direction());
        if sine.abs() < PARALLEL_SINE {
            return None;
        }
        let across = (second.origin() - first.origin()).perp_dot(second.direction()) / sine;
        let vertex = first.at(across);
        let first_ray = if reversed {
            -first.direction()
        } else {
            first.direction()
        };
        let signed = first_ray
            .perp_dot(second.direction())
            .atan2(first_ray.dot(second.direction()));
        let toward_segments = [(first, first_ray), (second, second.direction())]
            .iter()
            .filter_map(|(line, ray)| Some((line.middle()? - vertex).dot(*ray)))
            .sum::<f64>();
        let flip = if toward_segments < 0.0 { -1.0 } else { 1.0 };
        Some(Self {
            vertex,
            start: first_ray * flip,
            signed,
        })
    }

    fn ray(&self, fraction: f64) -> Vector2 {
        Vector2::from_angle(self.start.to_angle() + self.signed * fraction)
    }
}

fn arc_polyline(center: Point2, radius: f64, start: f64, sweep: f64) -> Vec<Point2> {
    let segments = (sweep.abs() / ARC_STEP).ceil().max(MIN_ARC_SEGMENTS);
    (0..=segments as usize)
        .map(|index| center + Vector2::from_angle(start + sweep * index as f64 / segments) * radius)
        .collect()
}

fn arc_reaching(
    center: Point2,
    radius: f64,
    (start, sweep): (f64, f64),
    at: Point2,
) -> Option<Vec<Point2>> {
    let turning = if sweep < 0.0 { -1.0 } else { 1.0 };
    let beyond_start = (((at - center).to_angle() - start) * turning).rem_euclid(TAU);
    let past_end = beyond_start - sweep.abs();
    if past_end <= 0.0 {
        return None;
    }
    let before_start = TAU - beyond_start;
    Some(if past_end <= before_start {
        arc_polyline(center, radius, start + sweep, turning * past_end)
    } else {
        arc_polyline(center, radius, start, -turning * before_start)
    })
}

fn extended_to(start: Point2, end: Point2, at: Point2) -> Option<[Point2; 2]> {
    let along = (end - start).try_normalize()?;
    let (reach, length) = ((at - start).dot(along), (end - start).dot(along));
    if reach < 0.0 {
        Some([start, at])
    } else if reach > length {
        Some([end, at])
    } else {
        None
    }
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
    placed: Option<Point2>,
) -> Option<DimensionLayout> {
    let offset = DIMENSION_OFFSET + LANE_SPACING * lane as f64;
    match (*measured, placed) {
        (Measured::Points(a, b), Some(at)) => placed_points_layout(screen, a, b, at),
        (Measured::Points(a, b), None) => points_layout(screen, a, b, centre, offset),
        (Measured::Aligned { from, to, along }, Some(at)) => {
            placed_aligned_layout(screen, from, to, along, at)
        }
        (Measured::Aligned { from, to, along }, None) => {
            aligned_layout(screen, from, to, along, centre, offset)
        }
        (Measured::PointToLine(point, line), placed) => led_to(
            point_to_line_layout(screen, point, line, centre)?,
            screen,
            placed,
        ),
        (
            Measured::PointToCircle {
                point,
                center,
                radius,
            },
            placed,
        ) => led_to(
            point_to_circle_layout(screen, point, center, radius)?,
            screen,
            placed,
        ),
        (Measured::Angle(first, second, reversed), placed) => {
            angle_layout(screen, first, second, reversed, placed)
        }
        (
            Measured::Radius {
                center,
                radius,
                toward,
            },
            placed,
        ) => radius_layout(screen, center, radius, toward, placed),
        (
            Measured::Diameter {
                center,
                radius,
                toward,
            },
            placed,
        ) => diameter_layout(screen, center, radius, toward, placed),
        (Measured::AlongArc { arc, from_centre }, placed) => {
            along_arc_layout(screen, &arc, from_centre, placed)
        }
    }
}

fn led_to(
    mut layout: DimensionLayout,
    screen: &impl Screen,
    placed: Option<Point2>,
) -> Option<DimensionLayout> {
    let Some(at) = placed else {
        return Some(layout);
    };
    let label = screen.to_screen(at)?;
    layout.strokes.push(vec![layout.label, label]);
    layout.label = label;
    layout.label_side = Vector2::ZERO;
    layout.at = at;
    Some(layout)
}

fn along_arc_layout(
    screen: &impl Screen,
    arc: &ArcGeometry,
    from_centre: bool,
    placed: Option<Point2>,
) -> Option<DimensionLayout> {
    let middle = arc.point_at(arc.start_angle + arc.sweep / 2.0);
    let projector = Projector::new(screen, middle)?;
    let placed = placed.filter(|at| at.distance(arc.center) > projector.units(MIN_EXTENSION));
    let radius = placed.map_or(arc.radius + projector.units(DIMENSION_OFFSET), |at| {
        at.distance(arc.center)
    });
    let ray = |fraction: f64| Vector2::from_angle(arc.start_angle + arc.sweep * fraction);
    let dimension = arc_polyline(arc.center, radius, arc.start_angle, arc.sweep);
    let mut strokes = vec![projector.polyline(&dimension)?];
    if let Some(reaching) =
        placed.and_then(|at| arc_reaching(arc.center, radius, (arc.start_angle, arc.sweep), at))
    {
        strokes.push(projector.polyline(&reaching)?);
    }
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
    let (label_at, label_side) = match placed {
        Some(at) => (at, Vector2::ZERO),
        None => {
            let label_at = arc.center + bisector * radius;
            (label_at, projector.direction(label_at, bisector)?)
        }
    };
    Some(DimensionLayout {
        strokes,
        arrows: vec![
            projector.arrow(arc.center + ray(0.0) * radius, -ray(0.0).perp())?,
            projector.arrow(arc.center + ray(1.0) * radius, ray(1.0).perp())?,
        ],
        label: projector.point(label_at)?,
        label_side,
        at: label_at,
    })
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
        at: middle + offset,
    })
}

fn placed_points_layout(
    screen: &impl Screen,
    a: Point2,
    b: Point2,
    at: Point2,
) -> Option<DimensionLayout> {
    let middle = (a + b) / 2.0;
    let projector = Projector::new(screen, middle)?;
    let along = (b - a).try_normalize().unwrap_or(Vector2::X);
    let normal = along.perp();
    let reach = (at - middle).dot(normal);
    let (start, end) = (a + normal * reach, b + normal * reach);
    let mut strokes = Vec::new();
    if reach.abs() > projector.units(EXTENSION_GAP) {
        let side = normal * reach.signum();
        for point in [a, b] {
            let foot = point + normal * reach;
            strokes.push(projector.polyline(&[
                point + side * projector.units(EXTENSION_GAP),
                foot + side * projector.units(EXTENSION_OVERSHOOT),
            ])?);
        }
    }
    strokes.push(projector.polyline(&[start, end])?);
    if let Some(extension) = extended_to(start, end, at) {
        strokes.push(projector.polyline(&extension)?);
    }
    Some(DimensionLayout {
        strokes,
        arrows: vec![
            projector.arrow(start, -along)?,
            projector.arrow(end, along)?,
        ],
        label: projector.point(at)?,
        label_side: Vector2::ZERO,
        at,
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
        at: (start + end) / 2.0,
    })
}

fn placed_aligned_layout(
    screen: &impl Screen,
    from: Point2,
    to: Point2,
    along: Vector2,
    at: Point2,
) -> Option<DimensionLayout> {
    let projector = Projector::new(screen, (from + to) / 2.0)?;
    let side = along.try_normalize()?.perp();
    let level = at.dot(side);
    let foot = |point: Point2| point + side * (level - point.dot(side));
    let (start, end) = (foot(from), foot(to));
    let mut strokes = Vec::new();
    for (point, foot) in [(from, start), (to, end)] {
        let length = (foot - point).dot(side);
        if length.abs() > projector.units(EXTENSION_GAP) {
            let outward = side * length.signum();
            strokes.push(projector.polyline(&[
                point + outward * projector.units(EXTENSION_GAP),
                foot + outward * projector.units(EXTENSION_OVERSHOOT),
            ])?);
        }
    }
    strokes.push(projector.polyline(&[start, end])?);
    if let Some(extension) = extended_to(start, end, at) {
        strokes.push(projector.polyline(&extension)?);
    }
    let direction = (end - start).try_normalize().unwrap_or(along);
    Some(DimensionLayout {
        strokes,
        arrows: vec![
            projector.arrow(start, -direction)?,
            projector.arrow(end, direction)?,
        ],
        label: projector.point(at)?,
        label_side: Vector2::ZERO,
        at,
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
            at: point,
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
        at: (point + on_circle) / 2.0,
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
        let side = away_from(line.direction().perp(), point, centre);
        return Some(DimensionLayout {
            strokes,
            arrows: Vec::new(),
            label: projector.point(point)?,
            label_side: projector.direction(point, side)?,
            at: point,
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
        at: (point + foot) / 2.0,
    })
}

fn angle_layout(
    screen: &impl Screen,
    first: LineSpan,
    second: LineSpan,
    reversed: bool,
    placed: Option<Point2>,
) -> Option<DimensionLayout> {
    let Some(corner) = Corner::of(first, second, reversed) else {
        return led_to(parallel_layout(screen, first, second)?, screen, placed);
    };
    let vertex = corner.vertex;
    let projector = Projector::new(screen, vertex)?;
    let placed = placed.filter(|at| at.distance(vertex) > projector.units(MIN_EXTENSION));
    let radius = placed.map_or(projector.units(ANGLE_RADIUS), |at| at.distance(vertex));
    let (start_angle, signed) = (corner.start.to_angle(), corner.signed);
    let ray = |fraction: f64| corner.ray(fraction);
    let arc = arc_polyline(vertex, radius, start_angle, signed);
    let turning = signed.signum();
    let mut strokes = vec![projector.polyline(&arc)?];
    if let Some(reaching) =
        placed.and_then(|at| arc_reaching(vertex, radius, (start_angle, signed), at))
    {
        strokes.push(projector.polyline(&reaching)?);
    }
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
    let (label_at, label_side) = match placed {
        Some(at) => (at, Vector2::ZERO),
        None => {
            let label_at = vertex + bisector * radius;
            (label_at, projector.direction(label_at, bisector)?)
        }
    };
    Some(DimensionLayout {
        strokes,
        arrows: vec![
            projector.arrow(vertex + ray(0.0) * radius, -ray(0.0).perp() * turning)?,
            projector.arrow(vertex + ray(1.0) * radius, ray(1.0).perp() * turning)?,
        ],
        label: projector.point(label_at)?,
        label_side,
        at: label_at,
    })
}

fn parallel_middle(first: LineSpan, second: LineSpan) -> Option<Point2> {
    let candidates = [(first, second), (second, first)]
        .into_iter()
        .flat_map(|(from, to)| {
            from.ends()
                .into_iter()
                .map(move |end| (end, to.closest(end)))
        });
    let (end, across) =
        candidates.min_by(|a, b| a.0.distance(a.1).total_cmp(&b.0.distance(b.1)))?;
    Some((end + across) / 2.0)
}

fn parallel_layout(
    screen: &impl Screen,
    first: LineSpan,
    second: LineSpan,
) -> Option<DimensionLayout> {
    let middle = parallel_middle(first, second)?;
    let projector = Projector::new(screen, middle)?;
    Some(DimensionLayout {
        strokes: Vec::new(),
        arrows: Vec::new(),
        label: projector.point(middle)?,
        label_side: Vector2::ZERO,
        at: middle,
    })
}

fn radius_layout(
    screen: &impl Screen,
    center: Point2,
    radius: f64,
    toward: Vector2,
    placed: Option<Point2>,
) -> Option<DimensionLayout> {
    let toward = placed
        .and_then(|at| (at - center).try_normalize())
        .unwrap_or(toward);
    let on_curve = center + toward * radius;
    let projector = Projector::new(screen, on_curve)?;
    let (end, label_side) = match placed {
        Some(at) => (farther(center, at, on_curve), Vector2::ZERO),
        None => {
            let end = on_curve + toward * projector.units(RADIUS_OVERSHOOT);
            (end, projector.direction(end, toward)?)
        }
    };
    let label_at = placed.unwrap_or(end);
    Some(DimensionLayout {
        strokes: vec![projector.polyline(&[center, end])?],
        arrows: vec![projector.arrow(on_curve, toward)?],
        label: projector.point(label_at)?,
        label_side,
        at: label_at,
    })
}

fn diameter_layout(
    screen: &impl Screen,
    center: Point2,
    radius: f64,
    toward: Vector2,
    placed: Option<Point2>,
) -> Option<DimensionLayout> {
    let toward = placed
        .and_then(|at| (at - center).try_normalize())
        .unwrap_or(toward);
    let (near, far) = (center - toward * radius, center + toward * radius);
    let projector = Projector::new(screen, far)?;
    let (end, label_side) = match placed {
        Some(at) => (farther(center, at, far), Vector2::ZERO),
        None => {
            let end = far + toward * projector.units(RADIUS_OVERSHOOT);
            (end, projector.direction(end, toward)?)
        }
    };
    let label_at = placed.unwrap_or(end);
    Some(DimensionLayout {
        strokes: vec![projector.polyline(&[near, end])?],
        arrows: vec![
            projector.arrow(far, toward)?,
            projector.arrow(near, -toward)?,
        ],
        label: projector.point(label_at)?,
        label_side,
        at: label_at,
    })
}

fn farther(center: Point2, first: Point2, second: Point2) -> Point2 {
    if first.distance(center) >= second.distance(center) {
        first
    } else {
        second
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum GlyphKind {
    Horizontal,
    Vertical,
    Parallel,
    Perpendicular,
    Tangent,
    Curvature,
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
        Constraint::OnMinorAxis { point, .. } => vec![(point, GlyphKind::Perpendicular)],
        Constraint::Concentric(a, b) => on_each(GlyphKind::Concentric, [a, b]),
        Constraint::Collinear(a, b) => on_each(GlyphKind::Collinear, [a, b]),
        Constraint::Symmetric { first, second, .. } => {
            on_each(GlyphKind::Symmetric, [first, second])
        }
        Constraint::Fix { point, .. } => vec![(point, GlyphKind::Fix)],
        Constraint::Parallel(a, b) => on_each(GlyphKind::Parallel, [a, b]),
        Constraint::Perpendicular(a, b) => on_each(GlyphKind::Perpendicular, [a, b]),
        Constraint::Tangent(a, b) => on_each(GlyphKind::Tangent, [a, b]),
        Constraint::Curvature(a, b) => on_each(GlyphKind::Curvature, [a, b]),
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
        | Constraint::AxisDiameter { .. }
        | Constraint::Angle { .. }
        | Constraint::Radius { .. }
        | Constraint::Diameter { .. }
        | Constraint::ArcLength { .. }
        | Constraint::Sweep { .. }
        | Constraint::MajorRadius { .. }
        | Constraint::MinorRadius { .. }
        | Constraint::Rho { .. } => Vec::new(),
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
        Entity::Ellipse { .. } | Entity::EllipticalArc { .. } => {
            let shape = sketch.ellipse(entity)?;
            on_curve(
                shape.center,
                shape.point_at(shape.start + shape.sweep * ARC_GLYPH_FRACTION),
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

pub fn obstacles() -> Obstacles {
    Obstacles::new(OBSTACLE_CELL)
}

pub fn mostly_covered(labels: &Obstacles, label: &Footprint) -> bool {
    labels.covers_more_than(label, MOST_LABEL_COVER)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Thinning {
    Never,
    WhenCrowded,
}

pub fn place_glyphs(
    anchor: GlyphAnchor,
    count: usize,
    centre: Option<Vector2>,
    half: Vector2,
    blocked: &Obstacles,
    thinning: Thinning,
) -> Option<Vec<Vector2>> {
    let most_covered = match thinning {
        Thinning::Never => f64::INFINITY,
        Thinning::WhenCrowded => {
            if neighbourhood(anchor, count, centre, half)
                .is_some_and(|region| blocked.cells_covered_over(&region, FULL_CELL_SHARE))
            {
                return None;
            }
            count as f64
                * Footprint {
                    center: Vector2::ZERO,
                    half,
                }
                .area()
                * MOST_GLYPH_COVER
        }
    };
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
            return Some(positions);
        }
        if best.as_ref().is_none_or(|(least, _)| overlapping < *least) {
            best = Some((overlapping, positions));
        }
    }
    best.filter(|(least, _)| *least <= most_covered)
        .map(|(_, positions)| positions)
}

fn neighbourhood(
    anchor: GlyphAnchor,
    count: usize,
    centre: Option<Vector2>,
    half: Vector2,
) -> Option<Footprint> {
    let unshifted: Vec<Placement> = match anchor {
        GlyphAnchor::Point(_) => (0..POINT_GLYPH_QUADRANTS.len())
            .map(|quadrant| Placement {
                quadrant,
                ..Placement::default()
            })
            .collect(),
        GlyphAnchor::Segment(..) | GlyphAnchor::Curve { .. } => [false, true]
            .map(|flipped| Placement {
                flipped,
                ..Placement::default()
            })
            .to_vec(),
    };
    let mut positions = unshifted
        .into_iter()
        .flat_map(|placement| stacked(anchor, count, centre, placement));
    let first = positions.next()?;
    let (low, high) = positions.fold((first, first), |(low, high), at| {
        (low.min(at), high.max(at))
    });
    Some(Footprint {
        center: (low + high) / 2.0,
        half: (high - low) / 2.0 + half,
    })
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

fn shifts_within(anchor: GlyphAnchor, count: usize) -> Option<usize> {
    let extent = count.saturating_sub(1) as f64 * GLYPH_SPACING;
    let reach = match anchor {
        GlyphAnchor::Segment(start, end) => ((start.distance(end) - extent) / 2.0).max(0.0),
        GlyphAnchor::Curve { .. } => MAX_CURVE_GLYPH_SHIFT,
        GlyphAnchor::Point(_) => return None,
    };
    Some(((reach / GLYPH_SHIFT_STEP).floor() as usize).min(MAX_GLYPH_SHIFTS))
}

fn placements(anchor: GlyphAnchor, count: usize) -> impl Iterator<Item = Placement> {
    let within = shifts_within(anchor, count);
    let quadrants = within
        .is_none()
        .then_some(0..POINT_GLYPH_QUADRANTS.len())
        .into_iter()
        .flatten()
        .map(|quadrant| Placement {
            quadrant,
            ..Placement::default()
        });
    let shifted = within.into_iter().flat_map(|within| {
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
    use caditor_sketch::annotation::{CIRCLE_LEADER_DIRECTION, lanes, measured};

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
        place_glyphs(
            anchor,
            count,
            centre,
            GLYPH_HALF,
            &obstacles(),
            Thinning::Never,
        )
        .unwrap()
    }

    fn label_at(center: Vector2, size: Vector2) -> Obstacles {
        let mut obstacles = obstacles();
        obstacles.add(Footprint {
            center,
            half: size / 2.0,
        });
        obstacles
    }

    #[test]
    fn a_horizontal_distance_sits_above_the_line_away_from_the_sketch() {
        let measured = Measured::Points(Point2::ZERO, Point2::new(40.0, 0.0));
        let layout = layout(&measured, &Flat, Some(Point2::new(20.0, -10.0)), 0, None).unwrap();

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

        let below = self::layout(&measured, &Flat, Some(Point2::new(20.0, 10.0)), 0, None).unwrap();
        assert_close(below.label, Vector2::new(140.0, 328.0));
    }

    #[test]
    fn a_rotated_distance_is_parallel_to_the_measured_segment() {
        let measured = Measured::Points(Point2::ZERO, Point2::new(30.0, 40.0));
        let layout = layout(&measured, &Flat, None, 0, None).unwrap();

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
        let layout = layout(&measured, &Flat, None, 0, None).unwrap();

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
        let layout = layout(&measured, &Flat, None, 0, None).unwrap();

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
        let layout = layout(&measured, &Flat, None, 0, None).unwrap();

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
        let layout = layout(&measured, &Flat, None, 0, None).unwrap();

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
        let layout = layout(&measured, &Flat, Some(Point2::new(15.0, -20.0)), 0, None).unwrap();

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
        let spanned = layout(&diameter, &Flat, None, 0, None).unwrap();

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
        let squarely = layout(&distance, &Flat, None, 0, None).unwrap();
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

        let along = layout(&length, &Flat, None, 0, None).unwrap();
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

        let turned = layout(&sweep, &Flat, None, 0, None).unwrap();
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
        let near = layout(measured[1].as_ref().unwrap(), &Flat, centre, 0, None).unwrap();
        let far = layout(measured[0].as_ref().unwrap(), &Flat, centre, 1, None).unwrap();
        assert_close(far.label - near.label, Vector2::new(25.0, LANE_SPACING));
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

        let moved = place_glyphs(anchor, 1, centre, GLYPH_HALF, &label, Thinning::Never).unwrap();
        assert_eq!(moved, vec![Vector2::new(14.0, 68.0)]);

        let tall = label_at(Vector2::new(14.0, 50.0), Vector2::new(20.0, 120.0));
        let flipped = place_glyphs(anchor, 1, centre, GLYPH_HALF, &tall, Thinning::Never).unwrap();
        assert_eq!(flipped, vec![Vector2::new(-14.0, 50.0)]);

        let everywhere = label_at(Vector2::new(0.0, 50.0), Vector2::new(200.0, 200.0));
        let least =
            place_glyphs(anchor, 1, centre, GLYPH_HALF, &everywhere, Thinning::Never).unwrap();
        assert_eq!(least, vec![Vector2::new(14.0, 50.0)]);

        let point = GlyphAnchor::Point(Vector2::new(5.0, 5.0));
        let upper_right = label_at(Vector2::new(15.0, -5.0), Vector2::new(10.0, 10.0));
        let left = place_glyphs(point, 1, None, GLYPH_HALF, &upper_right, Thinning::Never).unwrap();
        assert_eq!(left, vec![Vector2::new(-5.0, -5.0)]);
    }

    #[test]
    fn a_glyph_with_no_place_nearly_free_is_left_out_unless_kept() {
        let anchor = GlyphAnchor::Segment(Vector2::ZERO, Vector2::new(0.0, 100.0));
        let centre = Some(Vector2::new(50.0, 50.0));
        let mut both_sides = obstacles();
        for x in [-30.0, 30.0] {
            both_sides.add(Footprint {
                center: Vector2::new(x, 50.0),
                half: Vector2::new(11.0, 200.0),
            });
        }
        let everywhere = label_at(Vector2::new(0.0, 50.0), Vector2::new(200.0, 200.0));

        let grazed = place_glyphs(
            anchor,
            1,
            centre,
            GLYPH_HALF,
            &both_sides,
            Thinning::WhenCrowded,
        );
        let crowded = place_glyphs(
            anchor,
            1,
            centre,
            GLYPH_HALF,
            &everywhere,
            Thinning::WhenCrowded,
        );
        let kept = place_glyphs(anchor, 1, centre, GLYPH_HALF, &everywhere, Thinning::Never);

        assert_eq!(grazed, Some(vec![Vector2::new(14.0, 50.0)]));
        assert_eq!(crowded, None);
        assert_eq!(kept, Some(vec![Vector2::new(14.0, 50.0)]));
    }

    #[test]
    fn a_label_is_mostly_covered_once_others_cover_over_half_of_it() {
        let label = Footprint {
            center: Vector2::ZERO,
            half: Vector2::new(20.0, 10.0),
        };
        let mut others = obstacles();
        others.add(Footprint {
            center: Vector2::new(30.0, 0.0),
            half: Vector2::new(20.0, 10.0),
        });

        assert!(!mostly_covered(&others, &label));

        others.add(Footprint {
            center: Vector2::new(-15.0, 0.0),
            half: Vector2::new(10.0, 10.0),
        });

        assert!(mostly_covered(&others, &label));
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

    #[test]
    fn a_placed_distance_runs_its_dimension_line_through_the_label_and_turns_with_the_line() {
        let measured = Measured::Points(Point2::ZERO, Point2::new(40.0, 0.0));
        let frame = label_frame(&measured).unwrap();
        let placed = frame.place(Vector2::new(30.0, 12.0));

        let layout = layout(&measured, &Flat, None, 0, Some(placed)).unwrap();

        assert_close(placed, Point2::new(50.0, 12.0));
        assert_close(layout.at, placed);
        assert_close(layout.label, Vector2::new(200.0, 276.0));
        assert_eq!(layout.label_side, Vector2::ZERO);
        assert_close(layout.arrows[0].tip, Vector2::new(100.0, 276.0));
        assert_close(layout.arrows[1].tip, Vector2::new(180.0, 276.0));
        assert_eq!(
            layout.strokes.last(),
            Some(&vec![
                Vector2::new(180.0, 276.0),
                Vector2::new(200.0, 276.0)
            ])
        );

        let turned = Measured::Points(Point2::ZERO, Point2::new(0.0, 40.0));
        let turned_frame = label_frame(&turned).unwrap();
        let moved = turned_frame.place(Vector2::new(30.0, 12.0));
        assert_close(moved, Point2::new(-12.0, 50.0));
        assert_close(turned_frame.offset_of(moved), Vector2::new(30.0, 12.0));
    }

    #[test]
    fn a_placed_radius_points_its_leader_at_the_label_inside_or_outside_the_circle() {
        let measured = Measured::Radius {
            center: Point2::new(10.0, 10.0),
            radius: 5.0,
            toward: CIRCLE_LEADER_DIRECTION,
        };

        let outside = layout(&measured, &Flat, None, 0, Some(Point2::new(10.0, 30.0))).unwrap();
        let inside = layout(&measured, &Flat, None, 0, Some(Point2::new(10.0, 12.0))).unwrap();

        assert_close(outside.label, Vector2::new(120.0, 240.0));
        assert_close(outside.arrows[0].tip, Vector2::new(120.0, 270.0));
        assert_close(outside.arrows[0].direction, Vector2::new(0.0, -1.0));
        assert_eq!(
            outside.strokes,
            vec![vec![Vector2::new(120.0, 280.0), Vector2::new(120.0, 240.0)]]
        );
        assert_close(inside.label, Vector2::new(120.0, 276.0));
        assert_eq!(
            inside.strokes,
            vec![vec![Vector2::new(120.0, 280.0), Vector2::new(120.0, 270.0)]]
        );
    }

    #[test]
    fn a_placed_angle_takes_the_labels_radius_and_reaches_it_past_the_sweep() {
        let mut sketch = Sketch::new(Plane::XY);
        let first = sketch.add_line(Point2::ZERO, Point2::new(40.0, 0.0));
        let second = sketch.add_line(Point2::ZERO, Point2::new(30.0, 30.0));
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

        let layout = layout(&measured, &Flat, None, 0, Some(Point2::new(0.0, 20.0))).unwrap();

        assert_close(layout.label, Vector2::new(100.0, 260.0));
        assert_close(layout.arrows[0].tip, Vector2::new(140.0, 300.0));
        assert_eq!(layout.strokes.len(), 2);
        let reaching = &layout.strokes[1];
        assert_close(*reaching.last().unwrap(), Vector2::new(100.0, 260.0));
        assert!(
            reaching
                .iter()
                .all(|point| (point.distance(Vector2::new(100.0, 300.0)) - 40.0).abs() < 1e-9)
        );
    }
}
