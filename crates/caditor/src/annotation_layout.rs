use std::f64::consts::{FRAC_1_SQRT_2, PI};

use caditor_geometry::{Point2, Vector2};
use caditor_sketch::{Constraint, Entity, EntityId, Reference, Sketch};

use crate::snap::Screen;

const DIMENSION_OFFSET: f64 = 28.0;
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
}

pub fn measured(sketch: &Sketch, constraint: &Constraint) -> Option<Measured> {
    match *constraint {
        Constraint::Distance { from, to, .. } => match (sketch.point(from), sketch.point(to)) {
            (Some(a), Some(b)) => Some(Measured::Points(a, b)),
            (Some(point), None) => point_to_curve(sketch, point, to),
            (None, Some(point)) => point_to_curve(sketch, point, from),
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
) -> Option<DimensionLayout> {
    match *measured {
        Measured::Points(a, b) => points_layout(screen, a, b, centre),
        Measured::Aligned { from, to, along } => aligned_layout(screen, from, to, along, centre),
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
    }
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
) -> Option<DimensionLayout> {
    let middle = (a + b) / 2.0;
    let projector = Projector::new(screen, middle)?;
    let along = (b - a).try_normalize().unwrap_or(Vector2::X);
    let normal = away_from(along.perp(), middle, centre);
    let offset = normal * projector.units(DIMENSION_OFFSET);
    let extension = |point: Point2| {
        projector.polyline(&[
            point + normal * projector.units(EXTENSION_GAP),
            point + normal * projector.units(DIMENSION_OFFSET + EXTENSION_OVERSHOOT),
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
) -> Option<DimensionLayout> {
    let middle = (from + to) / 2.0;
    let projector = Projector::new(screen, middle)?;
    let side = away_from(along.perp(), middle, centre);
    let reach = from.dot(side).max(to.dot(side)) + projector.units(DIMENSION_OFFSET);
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
        | Constraint::Diameter { .. } => Vec::new(),
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
        Entity::Spline { .. } => None,
    }
}

pub fn stack_glyphs(anchor: GlyphAnchor, count: usize, centre: Option<Vector2>) -> Vec<Vector2> {
    let centred = |base: Vector2, step: Vector2| {
        let middle = count.saturating_sub(1) as f64 / 2.0;
        (0..count)
            .map(|index| base + step * (index as f64 - middle))
            .collect()
    };
    match anchor {
        GlyphAnchor::Segment(start, end) => {
            let middle = (start + end) / 2.0;
            let along = (end - start).try_normalize().unwrap_or(Vector2::X);
            let normal = toward(along.perp(), middle, centre);
            centred(middle + normal * GLYPH_OFFSET, along * GLYPH_SPACING)
        }
        GlyphAnchor::Curve { at, outward } => {
            centred(at + outward * GLYPH_OFFSET, outward.perp() * GLYPH_SPACING)
        }
        GlyphAnchor::Point(at) => (0..count)
            .map(|index| at + POINT_GLYPH_OFFSET + Vector2::X * POINT_GLYPH_SPACING * index as f64)
            .collect(),
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

    #[test]
    fn a_horizontal_distance_sits_above_the_line_away_from_the_sketch() {
        let measured = Measured::Points(Point2::ZERO, Point2::new(40.0, 0.0));
        let layout = layout(&measured, &Flat, Some(Point2::new(20.0, -10.0))).unwrap();

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

        let below = self::layout(&measured, &Flat, Some(Point2::new(20.0, 10.0))).unwrap();
        assert_close(below.label, Vector2::new(140.0, 328.0));
    }

    #[test]
    fn a_rotated_distance_is_parallel_to_the_measured_segment() {
        let measured = Measured::Points(Point2::ZERO, Point2::new(30.0, 40.0));
        let layout = layout(&measured, &Flat, None).unwrap();

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
        let layout = layout(&measured, &Flat, None).unwrap();

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
        let layout = layout(&measured, &Flat, None).unwrap();

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
        let layout = layout(&measured, &Flat, None).unwrap();

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
        let layout = layout(&measured, &Flat, None).unwrap();

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
        let layout = layout(&measured, &Flat, Some(Point2::new(15.0, -20.0))).unwrap();

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
        let spanned = layout(&diameter, &Flat, None).unwrap();

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
        let squarely = layout(&distance, &Flat, None).unwrap();
        assert_close(squarely.arrows[0].tip, Vector2::new(130.0, 280.0));
        assert_close(squarely.arrows[1].tip, Vector2::new(150.0, 280.0));
        assert_close(squarely.label, Vector2::new(140.0, 280.0));
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
            glyphs_of(&sketch, &Constraint::Midpoint { point: lone, line }),
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
}
