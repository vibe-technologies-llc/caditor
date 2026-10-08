use std::f64::consts::TAU;

use caditor_geometry::{Point2, Vector2};
use caditor_sketch::{ArcGeometry, Constraint, Entity, EntityId, Sketch};

pub const POINT_TOLERANCE: f64 = 8.0;
pub const CURVE_TOLERANCE: f64 = 6.0;
const ON_CIRCLE_TOLERANCE: f64 = 1e-9;
const PARALLEL_TOLERANCE: f64 = 1e-12;

pub trait Screen {
    fn to_screen(&self, point: Point2) -> Option<Vector2>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Right,
    Top,
    Left,
    Bottom,
}

impl Side {
    const ALL: [Self; 4] = [Self::Right, Self::Top, Self::Left, Self::Bottom];

    fn outward(self) -> Vector2 {
        match self {
            Self::Right => Vector2::X,
            Self::Top => Vector2::Y,
            Self::Left => -Vector2::X,
            Self::Bottom => -Vector2::Y,
        }
    }

    pub fn is_level(self) -> bool {
        matches!(self, Self::Right | Self::Left)
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Right => "Right",
            Self::Top => "Top",
            Self::Left => "Left",
            Self::Bottom => "Bottom",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Pending(usize),
    Point(EntityId),
    Curve(EntityId),
    Extension(EntityId),
    Midpoint(EntityId),
    Quadrant {
        curve: EntityId,
        centre: EntityId,
        side: Side,
    },
    Tangent(EntityId),
    Intersection(EntityId, EntityId),
    Centre {
        outline: EntityId,
        corners: (EntityId, EntityId),
    },
}

impl Target {
    #[cfg(test)]
    pub fn min_ordered(self) -> Self {
        match self {
            Self::Intersection(a, b) => Self::Intersection(a.min(b), a.max(b)),
            other => other,
        }
    }

    pub fn label(self, sketch: &Sketch) -> String {
        match self {
            Self::Pending(_) => "Stop here".to_owned(),
            Self::Point(EntityId::ORIGIN) => "Origin".to_owned(),
            Self::Midpoint(line) => format!("Midpoint of {}", sketch.entity_label(line)),
            Self::Quadrant { curve, side, .. } => {
                format!("{} of {}", side.name(), sketch.entity_label(curve))
            }
            Self::Tangent(curve) => format!("Tangent to {}", sketch.entity_label(curve)),
            Self::Centre { outline, .. } => {
                format!("Centre of the outline of {}", sketch.entity_label(outline))
            }
            Self::Intersection(first, second) => format!(
                "Crossing of {} and {}",
                sketch.entity_label(first),
                sketch.entity_label(second)
            ),
            Self::Point(entity) | Self::Curve(entity) => {
                format!("On {}", sketch.entity_label(entity))
            }
            Self::Extension(line) => {
                format!("On the extension of {}", sketch.entity_label(line))
            }
        }
    }

    pub fn joins(self, point: EntityId) -> Vec<Constraint> {
        match self {
            Self::Pending(_) => Vec::new(),
            Self::Midpoint(curve) => vec![Constraint::Midpoint { point, curve }],
            Self::Quadrant {
                curve,
                centre,
                side,
            } => vec![
                Constraint::Coincident(point, curve),
                if side.is_level() {
                    Constraint::HorizontalPoints(point, centre)
                } else {
                    Constraint::VerticalPoints(point, centre)
                },
            ],
            Self::Centre {
                corners: (first, second),
                ..
            } => vec![Constraint::Symmetric {
                first,
                second,
                about: point,
            }],
            Self::Intersection(first, second) => vec![
                Constraint::Coincident(point, first),
                Constraint::Coincident(point, second),
            ],
            Self::Point(entity)
            | Self::Curve(entity)
            | Self::Extension(entity)
            | Self::Tangent(entity) => vec![Constraint::Coincident(point, entity)],
        }
    }

    fn touches(self, ignored: &[EntityId]) -> bool {
        let involved = match self {
            Self::Pending(_) => Vec::new(),
            Self::Quadrant { curve, centre, .. } => vec![curve, centre],
            Self::Centre {
                outline,
                corners: (first, second),
            } => vec![outline, first, second],
            Self::Intersection(first, second) => vec![first, second],
            Self::Point(entity)
            | Self::Curve(entity)
            | Self::Extension(entity)
            | Self::Midpoint(entity)
            | Self::Tangent(entity) => vec![entity],
        };
        involved.iter().any(|entity| ignored.contains(entity))
    }

    pub fn is_point_like(self) -> bool {
        match self {
            Self::Curve(_) | Self::Extension(_) => false,
            Self::Pending(_)
            | Self::Point(_)
            | Self::Midpoint(_)
            | Self::Quadrant { .. }
            | Self::Tangent(_)
            | Self::Intersection(..)
            | Self::Centre { .. } => true,
        }
    }

    pub fn second_entity(self) -> Option<EntityId> {
        match self {
            Self::Intersection(_, second) => Some(second),
            Self::Pending(_)
            | Self::Point(_)
            | Self::Curve(_)
            | Self::Extension(_)
            | Self::Midpoint(_)
            | Self::Quadrant { .. }
            | Self::Tangent(_)
            | Self::Centre { .. } => None,
        }
    }

    pub fn entity(self) -> Option<EntityId> {
        match self {
            Self::Pending(_) => None,
            Self::Point(entity)
            | Self::Curve(entity)
            | Self::Extension(entity)
            | Self::Midpoint(entity)
            | Self::Quadrant { curve: entity, .. }
            | Self::Tangent(entity)
            | Self::Intersection(entity, _)
            | Self::Centre {
                outline: entity, ..
            } => Some(entity),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Snapped {
    pub position: Point2,
    pub target: Target,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pointer {
    pub screen: Vector2,
    pub sketch: Point2,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Accept {
    Anything,
    Points,
    OnCircle { center: Point2, radius: f64 },
}

pub fn resolve(
    sketch: &Sketch,
    screen: &impl Screen,
    pointer: Pointer,
    pending: &[(usize, Point2)],
    accept: Accept,
    extended: &[EntityId],
    ignored: &[EntityId],
) -> Option<Snapped> {
    let pending = pending
        .iter()
        .map(|(index, position)| Snapped {
            position: *position,
            target: Target::Pending(*index),
        })
        .collect();
    let nearest = |candidates: Vec<Snapped>, tolerance: f64| {
        candidates
            .into_iter()
            .filter(|candidate| !candidate.target.touches(ignored))
            .filter_map(|candidate| {
                let offset = screen
                    .to_screen(candidate.position)?
                    .distance(pointer.screen);
                (offset <= tolerance).then_some((offset, candidate))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, candidate)| candidate)
    };
    let points = match accept {
        Accept::Anything | Accept::Points => points(sketch),
        Accept::OnCircle { center, radius } => points(sketch)
            .into_iter()
            .filter(|candidate| on_circle(center, radius, candidate.position))
            .collect(),
    };
    nearest(pending, POINT_TOLERANCE)
        .or_else(|| nearest(points, POINT_TOLERANCE))
        .or_else(|| match accept {
            Accept::Anything => nearest(midpoints(sketch), POINT_TOLERANCE)
                .or_else(|| nearest(centres(sketch), POINT_TOLERANCE))
                .or_else(|| nearest(intersections(sketch, screen, pointer), POINT_TOLERANCE))
                .or_else(|| nearest(quadrants(sketch), POINT_TOLERANCE)),
            Accept::Points | Accept::OnCircle { .. } => None,
        })
        .or_else(|| match accept {
            Accept::Anything => {
                nearest(curves(sketch, pointer.sketch), CURVE_TOLERANCE).or_else(|| {
                    nearest(
                        extensions(sketch, extended, pointer.sketch),
                        CURVE_TOLERANCE,
                    )
                })
            }
            Accept::Points => None,
            Accept::OnCircle { center, radius } => {
                nearest(crossings(sketch, center, radius), CURVE_TOLERANCE)
            }
        })
}

pub fn on_circle(center: Point2, radius: f64, point: Point2) -> bool {
    (point.distance(center) - radius).abs() <= ON_CIRCLE_TOLERANCE * radius.max(1.0)
}

pub fn points(sketch: &Sketch) -> Vec<Snapped> {
    let origin = Snapped {
        position: Point2::ZERO,
        target: Target::Point(EntityId::ORIGIN),
    };
    let drawn = sketch.entities().filter_map(|(id, entity)| match entity {
        Entity::Point(position) => Some(Snapped {
            position: *position,
            target: Target::Point(id),
        }),
        Entity::Line { .. }
        | Entity::Circle { .. }
        | Entity::Arc { .. }
        | Entity::Spline { .. } => None,
    });
    std::iter::once(origin).chain(drawn).collect()
}

const NEAR_CURVE_PIXELS: f64 = 4.0 * POINT_TOLERANCE;
const MAX_CROSSING_CURVES: usize = 12;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Geometry {
    Infinite(Point2, Vector2),
    Segment(Point2, Point2),
    Circle(Point2, f64),
    Arc(ArcGeometry),
}

fn geometry(sketch: &Sketch, id: EntityId) -> Option<Geometry> {
    if id == EntityId::HORIZONTAL_AXIS {
        return Some(Geometry::Infinite(Point2::ZERO, Vector2::X));
    }
    if id == EntityId::VERTICAL_AXIS {
        return Some(Geometry::Infinite(Point2::ZERO, Vector2::Y));
    }
    match sketch.entity(id)? {
        Entity::Line { .. } => sketch
            .line_endpoints(id)
            .map(|(start, end)| Geometry::Segment(start, end)),
        Entity::Circle { .. } => sketch
            .circle(id)
            .map(|(center, radius)| Geometry::Circle(center, radius)),
        Entity::Arc { .. } => sketch.arc(id).map(Geometry::Arc),
        Entity::Point(_) | Entity::Spline { .. } => None,
    }
}

fn straight(geometry: Geometry) -> Option<(Point2, Vector2, bool)> {
    match geometry {
        Geometry::Infinite(through, along) => Some((through, along, false)),
        Geometry::Segment(start, end) => Some((start, end - start, true)),
        Geometry::Circle(..) | Geometry::Arc(_) => None,
    }
}

fn round(geometry: Geometry) -> Option<(Point2, f64, Option<ArcGeometry>)> {
    match geometry {
        Geometry::Circle(center, radius) => Some((center, radius, None)),
        Geometry::Arc(arc) => Some((arc.center, arc.radius, Some(arc))),
        Geometry::Infinite(..) | Geometry::Segment(..) => None,
    }
}

fn hits(first: Geometry, second: Geometry) -> Vec<Point2> {
    let on_arc = |arc: Option<ArcGeometry>, position: &Point2| {
        arc.is_none_or(|arc| within_sweep(&arc, *position))
    };
    match (
        straight(first),
        straight(second),
        round(first),
        round(second),
    ) {
        (Some((through, along, bounded)), Some((origin, direction, other_bounded)), _, _) => {
            let Some((fraction, position)) = straight_crossing(through, along, origin, direction)
            else {
                return Vec::new();
            };
            let own = (position - through).dot(along) / along.length_squared();
            let inside = |bounded: bool, fraction: f64| !bounded || (0.0..=1.0).contains(&fraction);
            (inside(bounded, own) && inside(other_bounded, fraction))
                .then_some(position)
                .into_iter()
                .collect()
        }
        (Some((through, along, bounded)), None, None, Some((center, radius, arc)))
        | (None, Some((through, along, bounded)), Some((center, radius, arc)), None) => {
            line_crossings(through, along, center, radius)
                .into_iter()
                .filter(|(fraction, _)| !bounded || (0.0..=1.0).contains(fraction))
                .map(|(_, position)| position)
                .filter(|position| on_arc(arc, position))
                .collect()
        }
        (None, None, Some((center, radius, arc)), Some((other, other_radius, other_arc))) => {
            circle_crossings(center, radius, other, other_radius)
                .into_iter()
                .filter(|position| on_arc(arc, position) && on_arc(other_arc, position))
                .collect()
        }
        _ => Vec::new(),
    }
}

fn intersections(sketch: &Sketch, screen: &impl Screen, pointer: Pointer) -> Vec<Snapped> {
    let mut near: Vec<(f64, EntityId, Option<Geometry>)> = curves(sketch, pointer.sketch)
        .into_iter()
        .filter_map(|candidate| {
            let Target::Curve(id) = candidate.target else {
                return None;
            };
            let distance = screen
                .to_screen(candidate.position)?
                .distance(pointer.screen);
            (distance <= NEAR_CURVE_PIXELS).then_some((distance, id, geometry(sketch, id)))
        })
        .collect();
    near.sort_by(|a, b| a.0.total_cmp(&b.0));
    near.truncate(MAX_CROSSING_CURVES);
    let mut found = Vec::new();
    for (index, (_, first, first_geometry)) in near.iter().enumerate() {
        for (_, second, second_geometry) in near.iter().skip(index + 1) {
            let (low, high) = if first <= second {
                (*first, *second)
            } else {
                (*second, *first)
            };
            let crossed = match (first_geometry, second_geometry) {
                (Some(first_geometry), Some(second_geometry)) => {
                    hits(*first_geometry, *second_geometry)
                }
                (None, Some(_)) => sketch.spline_crossings(*first, *second),
                (Some(_), None) | (None, None) => sketch.spline_crossings(*second, *first),
            };
            found.extend(crossed.into_iter().map(|position| Snapped {
                position,
                target: Target::Intersection(low, high),
            }));
        }
    }
    found
}

fn midpoints(sketch: &Sketch) -> Vec<Snapped> {
    sketch
        .entities()
        .filter_map(|(id, entity)| {
            let position = match entity {
                Entity::Line { .. } => {
                    let (start, end) = sketch.line_endpoints(id)?;
                    start.midpoint(end)
                }
                Entity::Arc { .. } => {
                    let arc = sketch.arc(id)?;
                    arc.point_at(arc.start_angle + arc.sweep / 2.0)
                }
                Entity::Point(_) | Entity::Circle { .. } | Entity::Spline { .. } => return None,
            };
            Some(Snapped {
                position,
                target: Target::Midpoint(id),
            })
        })
        .collect()
}

fn quadrants(sketch: &Sketch) -> Vec<Snapped> {
    sketch
        .entities()
        .flat_map(|(curve, entity)| {
            let (centre, arc) = match *entity {
                Entity::Circle { center, .. } => (center, None),
                Entity::Arc { center, .. } => (center, sketch.arc(curve)),
                Entity::Point(_) | Entity::Line { .. } | Entity::Spline { .. } => {
                    return Vec::new();
                }
            };
            let Some((position, radius)) = sketch.circle(curve) else {
                return Vec::new();
            };
            Side::ALL
                .into_iter()
                .map(|side| (side, position + side.outward() * radius))
                .filter(|(_, at)| arc.is_none_or(|arc| within_sweep(&arc, *at)))
                .map(|(side, at)| Snapped {
                    position: at,
                    target: Target::Quadrant {
                        curve,
                        centre,
                        side,
                    },
                })
                .collect()
        })
        .collect()
}

pub fn tangents_from(
    sketch: &Sketch,
    screen: &impl Screen,
    pointer: Pointer,
    from: Point2,
) -> Option<Snapped> {
    let pointed = |at: Point2| Some(screen.to_screen(at)?.distance(pointer.screen));
    sketch
        .entities()
        .filter_map(|(curve, entity)| {
            let arc = match entity {
                Entity::Circle { .. } => None,
                Entity::Arc { .. } => Some(sketch.arc(curve)?),
                Entity::Point(_) | Entity::Line { .. } | Entity::Spline { .. } => return None,
            };
            let (centre, radius) = sketch.circle(curve)?;
            Some((curve, centre, radius, arc))
        })
        .flat_map(|(curve, centre, radius, arc)| {
            touching(from, centre, radius)
                .into_iter()
                .filter(move |at| arc.is_none_or(|arc| within_sweep(&arc, *at)))
                .map(move |position| Snapped {
                    position,
                    target: Target::Tangent(curve),
                })
        })
        .filter_map(|candidate| {
            let offset = pointed(candidate.position)?;
            (offset <= POINT_TOLERANCE).then_some((offset, candidate))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, candidate)| candidate)
}

fn touching(from: Point2, centre: Point2, radius: f64) -> Vec<Point2> {
    let away = from - centre;
    let distance = away.length();
    if radius <= 0.0 || distance <= radius * (1.0 + TANGENT_CLEARANCE) {
        return Vec::new();
    }
    let toward = away / distance;
    let along = radius * radius / distance;
    let across = radius * (1.0 - (radius / distance).powi(2)).sqrt();
    [1.0, -1.0]
        .into_iter()
        .map(|sign| centre + toward * along + toward.perp() * across * sign)
        .collect()
}

const TANGENT_CLEARANCE: f64 = 1e-6;

struct Corner {
    position: Point2,
    line: usize,
    at_end: bool,
}

struct OutlineLine {
    id: EntityId,
    start: (EntityId, Point2),
    end: (EntityId, Point2),
    round: Option<(Point2, f64)>,
}

fn outline_pieces(sketch: &Sketch) -> Vec<OutlineLine> {
    sketch
        .entities()
        .filter_map(|(id, entity)| match *entity {
            Entity::Line { start, end } => {
                let (from, to) = sketch.line_endpoints(id)?;
                Some(OutlineLine {
                    id,
                    start: (start, from),
                    end: (end, to),
                    round: None,
                })
            }
            Entity::Arc { start, end, .. } => {
                let arc = sketch.arc(id)?;
                Some(OutlineLine {
                    id,
                    start: (start, sketch.point(start)?),
                    end: (end, sketch.point(end)?),
                    round: Some((arc.center, arc.radius)),
                })
            }
            Entity::Point(_) | Entity::Circle { .. } | Entity::Spline { .. } => None,
        })
        .collect()
}

fn centres(sketch: &Sketch) -> Vec<Snapped> {
    let lines = outline_pieces(sketch);
    let extent = lines
        .iter()
        .flat_map(|line| [line.start.1, line.end.1])
        .fold(1.0_f64, |most, point| most.max(point.abs().max_element()));
    let tolerance = JOINED_CORNER_TOLERANCE * extent;
    let partners = corner_partners(&lines, tolerance);
    let mut visited = vec![false; lines.len()];
    let mut found = Vec::new();
    for first in 0..lines.len() {
        if visited.get(first).copied().unwrap_or(true) {
            continue;
        }
        let Some(outline) = closed_outline(&lines, &partners, first, &mut visited) else {
            continue;
        };
        let Some(id) = lines.get(first).map(|line| line.id) else {
            continue;
        };
        found.extend(
            symmetric_centre(&lines, &outline, tolerance).map(|(position, corners)| Snapped {
                position,
                target: Target::Centre {
                    outline: id,
                    corners,
                },
            }),
        );
    }
    found
}

const JOINED_CORNER_TOLERANCE: f64 = 1e-7;
const MAX_OUTLINE_LINES: usize = 64;

fn corner_partners(lines: &[OutlineLine], tolerance: f64) -> Vec<Option<(usize, bool)>> {
    let mut corners: Vec<Corner> = lines
        .iter()
        .enumerate()
        .flat_map(|(index, line)| {
            [(line.start.1, false), (line.end.1, true)].map(|(position, at_end)| Corner {
                position,
                line: index,
                at_end,
            })
        })
        .collect();
    corners.sort_by(|a, b| a.position.x.total_cmp(&b.position.x));
    let mut meeting: Vec<Vec<usize>> = vec![Vec::new(); corners.len()];
    for (index, corner) in corners.iter().enumerate() {
        for (offset, other) in corners.iter().enumerate().skip(index + 1) {
            if other.position.x - corner.position.x > tolerance {
                break;
            }
            if other.position.distance(corner.position) <= tolerance {
                if let Some(list) = meeting.get_mut(index) {
                    list.push(offset);
                }
                if let Some(list) = meeting.get_mut(offset) {
                    list.push(index);
                }
            }
        }
    }
    let mut partners = vec![None; 2 * lines.len()];
    for (corner, others) in corners.iter().zip(&meeting) {
        let [other] = others.as_slice() else {
            continue;
        };
        let Some(other) = corners.get(*other) else {
            continue;
        };
        if other.line != corner.line
            && let Some(partner) = partners.get_mut(2 * corner.line + usize::from(corner.at_end))
        {
            *partner = Some((other.line, other.at_end));
        }
    }
    partners
}

fn closed_outline(
    lines: &[OutlineLine],
    partners: &[Option<(usize, bool)>],
    first: usize,
    visited: &mut [bool],
) -> Option<Vec<OutlineStep>> {
    let mut steps = Vec::new();
    let (mut line, mut leaving_end) = (first, true);
    loop {
        if let Some(seen) = visited.get_mut(line) {
            *seen = true;
        }
        let current = lines.get(line)?;
        steps.push(OutlineStep {
            corner: if leaving_end {
                current.end
            } else {
                current.start
            },
            piece: line,
        });
        if steps.len() > MAX_OUTLINE_LINES {
            return None;
        }
        let (next, arriving_end) = (*partners.get(2 * line + usize::from(leaving_end))?)?;
        if next == first {
            return (!arriving_end).then_some(steps);
        }
        if visited.get(next).copied().unwrap_or(true) {
            return None;
        }
        (line, leaving_end) = (next, !arriving_end);
    }
}

struct OutlineStep {
    corner: (EntityId, Point2),
    piece: usize,
}

fn symmetric_centre(
    lines: &[OutlineLine],
    steps: &[OutlineStep],
    tolerance: f64,
) -> Option<(Point2, (EntityId, EntityId))> {
    let count = steps.len();
    if count < 4 || !count.is_multiple_of(2) {
        return None;
    }
    let half = count / 2;
    let (first, opposite) = (steps.first()?.corner, steps.get(half)?.corner);
    let centre = first.1.midpoint(opposite.1);
    let mirrored = |a: Point2, b: Point2| a.midpoint(b).distance(centre) <= tolerance;
    let symmetric = steps
        .iter()
        .zip(steps.iter().skip(half))
        .all(|(step, across)| {
            let pieces = lines.get(step.piece).zip(lines.get(across.piece));
            let rounds_match =
                pieces.is_some_and(|(piece, other)| match (piece.round, other.round) {
                    (None, None) => true,
                    (Some((centre_a, radius_a)), Some((centre_b, radius_b))) => {
                        mirrored(centre_a, centre_b) && (radius_a - radius_b).abs() <= tolerance
                    }
                    (Some(_), None) | (None, Some(_)) => false,
                });
            mirrored(step.corner.1, across.corner.1) && rounds_match
        });
    let spread = first.1.distance(opposite.1) > tolerance;
    (symmetric && spread).then_some((centre, (first.0, opposite.0)))
}

fn curves(sketch: &Sketch, at: Point2) -> Vec<Snapped> {
    let axes = [
        Snapped {
            position: Point2::new(at.x, 0.0),
            target: Target::Curve(EntityId::HORIZONTAL_AXIS),
        },
        Snapped {
            position: Point2::new(0.0, at.y),
            target: Target::Curve(EntityId::VERTICAL_AXIS),
        },
    ];
    let drawn = sketch.entities().filter_map(|(id, entity)| {
        Some(Snapped {
            position: closest_on(sketch, id, entity, at)?,
            target: Target::Curve(id),
        })
    });
    axes.into_iter().chain(drawn).collect()
}

fn extensions(sketch: &Sketch, lines: &[EntityId], at: Point2) -> Vec<Snapped> {
    lines
        .iter()
        .filter_map(|line| {
            let (start, end) = sketch.line_endpoints(*line)?;
            let along = end - start;
            let length_squared = along.length_squared();
            if length_squared == 0.0 {
                return None;
            }
            let fraction = (at - start).dot(along) / length_squared;
            (!(0.0..=1.0).contains(&fraction)).then(|| Snapped {
                position: start + along * fraction,
                target: Target::Extension(*line),
            })
        })
        .collect()
}

pub fn extension_guide(sketch: &Sketch, line: EntityId, to: Point2) -> Option<[Point2; 2]> {
    let (start, end) = sketch.line_endpoints(line)?;
    let nearer = if start.distance_squared(to) <= end.distance_squared(to) {
        start
    } else {
        end
    };
    Some([nearer, to])
}

fn closest_on(sketch: &Sketch, id: EntityId, entity: &Entity, at: Point2) -> Option<Point2> {
    match entity {
        Entity::Line { .. } => {
            let (start, end) = sketch.line_endpoints(id)?;
            Some(closest_on_segment(start, end, at))
        }
        Entity::Circle { .. } => {
            let (center, radius) = sketch.circle(id)?;
            closest_on_circle(center, radius, at)
        }
        Entity::Arc { .. } => {
            let arc = sketch.arc(id)?;
            let position = closest_on_circle(arc.center, arc.radius, at)?;
            within_sweep(&arc, position).then_some(position)
        }
        Entity::Spline { .. } => sketch.closest_on_curve(id, at),
        Entity::Point(_) => None,
    }
}

fn crossings(sketch: &Sketch, center: Point2, radius: f64) -> Vec<Snapped> {
    let axes = [
        (EntityId::HORIZONTAL_AXIS, Point2::ZERO, Point2::X),
        (EntityId::VERTICAL_AXIS, Point2::ZERO, Point2::Y),
    ]
    .into_iter()
    .flat_map(|(axis, through, along)| {
        line_crossings(through, along, center, radius)
            .into_iter()
            .map(move |(_, position)| Snapped {
                position,
                target: Target::Curve(axis),
            })
    });
    let drawn = sketch.entities().flat_map(|(id, entity)| {
        crossings_with(sketch, id, entity, center, radius)
            .into_iter()
            .map(move |position| Snapped {
                position,
                target: Target::Curve(id),
            })
    });
    axes.chain(drawn).collect()
}

fn crossings_with(
    sketch: &Sketch,
    id: EntityId,
    entity: &Entity,
    center: Point2,
    radius: f64,
) -> Vec<Point2> {
    match entity {
        Entity::Line { .. } => sketch
            .line_endpoints(id)
            .map(|(start, end)| {
                line_crossings(start, end - start, center, radius)
                    .into_iter()
                    .filter(|(along, _)| (0.0..=1.0).contains(along))
                    .map(|(_, position)| position)
                    .collect()
            })
            .unwrap_or_default(),
        Entity::Circle { .. } => sketch
            .circle(id)
            .map(|(other, other_radius)| circle_crossings(center, radius, other, other_radius))
            .unwrap_or_default(),
        Entity::Arc { .. } => sketch
            .arc(id)
            .map(|arc| {
                circle_crossings(center, radius, arc.center, arc.radius)
                    .into_iter()
                    .filter(|position| within_sweep(&arc, *position))
                    .collect()
            })
            .unwrap_or_default(),
        Entity::Spline { .. } => sketch.circle_crossings(id, center, radius),
        Entity::Point(_) => Vec::new(),
    }
}

fn line_crossings(
    through: Point2,
    along: Vector2,
    center: Point2,
    radius: f64,
) -> Vec<(f64, Point2)> {
    let length_squared = along.length_squared();
    if length_squared == 0.0 {
        return Vec::new();
    }
    let foot = (center - through).dot(along) / length_squared;
    let closest = through + along * foot;
    let half_chord_squared = radius * radius - closest.distance_squared(center);
    if half_chord_squared < 0.0 {
        return Vec::new();
    }
    let half = half_chord_squared.sqrt() / length_squared.sqrt();
    [foot - half, foot + half]
        .into_iter()
        .map(|parameter| (parameter, through + along * parameter))
        .collect()
}

fn circle_crossings(center: Point2, radius: f64, other: Point2, other_radius: f64) -> Vec<Point2> {
    let between = other - center;
    let distance = between.length();
    if distance == 0.0
        || distance > radius + other_radius
        || distance < (radius - other_radius).abs()
    {
        return Vec::new();
    }
    let along =
        (radius * radius - other_radius * other_radius + distance * distance) / (2.0 * distance);
    let half_chord = (radius * radius - along * along).max(0.0).sqrt();
    let direction = between / distance;
    let middle = center + direction * along;
    let across = direction.perp() * half_chord;
    vec![middle + across, middle - across]
}

pub fn crossing_along(
    sketch: &Sketch,
    target: Target,
    through: Point2,
    along: Vector2,
    near: Point2,
) -> Option<Point2> {
    let (curve, extended) = match target {
        Target::Curve(curve) => (curve, false),
        Target::Extension(line) => (line, true),
        Target::Pending(_)
        | Target::Point(_)
        | Target::Midpoint(_)
        | Target::Quadrant { .. }
        | Target::Tangent(_)
        | Target::Intersection(..)
        | Target::Centre { .. } => return None,
    };
    let on_part = |fraction: f64| (0.0..=1.0).contains(&fraction) != extended;
    let crossings: Vec<Point2> = if curve == EntityId::HORIZONTAL_AXIS {
        straight_crossing(through, along, Point2::ZERO, Vector2::X)
            .map(|(_, position)| position)
            .into_iter()
            .collect()
    } else if curve == EntityId::VERTICAL_AXIS {
        straight_crossing(through, along, Point2::ZERO, Vector2::Y)
            .map(|(_, position)| position)
            .into_iter()
            .collect()
    } else {
        match sketch.entity(curve)? {
            Entity::Line { .. } => {
                let (start, end) = sketch.line_endpoints(curve)?;
                straight_crossing(through, along, start, end - start)
                    .filter(|(fraction, _)| on_part(*fraction))
                    .map(|(_, position)| position)
                    .into_iter()
                    .collect()
            }
            Entity::Circle { .. } => {
                let (center, radius) = sketch.circle(curve)?;
                line_crossings(through, along, center, radius)
                    .into_iter()
                    .map(|(_, position)| position)
                    .collect()
            }
            Entity::Arc { .. } => {
                let arc = sketch.arc(curve)?;
                line_crossings(through, along, arc.center, arc.radius)
                    .into_iter()
                    .map(|(_, position)| position)
                    .filter(|position| within_sweep(&arc, *position))
                    .collect()
            }
            Entity::Spline { .. } => {
                let reach = sketch.spline(curve).map_or(0.0, |spline| {
                    spline
                        .control_points()
                        .iter()
                        .map(|point| point.distance(through))
                        .fold(0.0, f64::max)
                });
                let along = along.try_normalize()?;
                sketch.segment_crossings(
                    curve,
                    through - along * 2.0 * reach,
                    through + along * 2.0 * reach,
                )
            }
            Entity::Point(_) => Vec::new(),
        }
    };
    crossings.into_iter().min_by(|a, b| {
        a.distance_squared(near)
            .total_cmp(&b.distance_squared(near))
    })
}

fn straight_crossing(
    through: Point2,
    along: Vector2,
    origin: Point2,
    direction: Vector2,
) -> Option<(f64, Point2)> {
    let denominator = along.perp_dot(direction);
    if denominator.abs() <= PARALLEL_TOLERANCE * along.length() * direction.length() {
        return None;
    }
    let fraction = along.perp_dot(through - origin) / denominator;
    Some((fraction, origin + direction * fraction))
}

fn within_sweep(arc: &ArcGeometry, position: Point2) -> bool {
    let offset = position - arc.center;
    let turned = (offset.y.atan2(offset.x) - arc.start_angle).rem_euclid(TAU);
    turned <= arc.sweep
}

pub fn closest_on_segment(start: Point2, end: Point2, at: Point2) -> Point2 {
    let along = end - start;
    let length_squared = along.length_squared();
    if length_squared == 0.0 {
        return start;
    }
    let fraction = ((at - start).dot(along) / length_squared).clamp(0.0, 1.0);
    start + along * fraction
}

pub fn closest_on_circle(center: Point2, radius: f64, at: Point2) -> Option<Point2> {
    let direction = (at - center).try_normalize()?;
    Some(center + direction * radius)
}

#[cfg(test)]
pub mod tests {
    use std::f64::consts::PI;

    use caditor_geometry::Plane;

    use super::*;

    pub struct Scaled(pub f64);

    impl Screen for Scaled {
        fn to_screen(&self, point: Point2) -> Option<Vector2> {
            Some(Vector2::new(point.x, -point.y) * self.0)
        }
    }

    fn pointer_at(sketch: Point2) -> Pointer {
        Pointer {
            screen: Scaled(10.0).to_screen(sketch).unwrap(),
            sketch,
        }
    }

    fn resolve_at(sketch: &Sketch, at: Point2) -> Option<Snapped> {
        resolve(
            sketch,
            &Scaled(10.0),
            pointer_at(at),
            &[],
            Accept::Anything,
            &[],
            &[],
        )
    }

    fn point_of(sketch: &Sketch, curve: EntityId, index: usize) -> EntityId {
        sketch.entity(curve).unwrap().points()[index]
    }

    #[test]
    fn points_win_over_curves_within_their_tolerance() {
        let mut sketch = Sketch::new(Plane::XY);
        let line = sketch.add_line(Point2::new(10.0, 10.0), Point2::new(30.0, 10.0));
        let start = point_of(&sketch, line, 0);

        let near_start = resolve_at(&sketch, Point2::new(10.7, 10.1)).unwrap();
        assert_eq!(near_start.target, Target::Point(start));
        assert_eq!(near_start.position, Point2::new(10.0, 10.0));

        let on_line = resolve_at(&sketch, Point2::new(16.0, 10.5)).unwrap();
        assert_eq!(on_line.target, Target::Curve(line));
        assert_eq!(on_line.position, Point2::new(16.0, 10.0));

        assert_eq!(resolve_at(&sketch, Point2::new(16.0, 10.7)), None);
        assert_eq!(resolve_at(&sketch, Point2::new(10.0, 10.9)), None);
    }

    #[test]
    fn the_middle_of_a_line_is_a_snap_target_ahead_of_the_line_but_not_of_points_or_circles() {
        let mut sketch = Sketch::new(Plane::XY);
        let line = sketch.add_line(Point2::new(10.0, 10.0), Point2::new(30.0, 10.0));

        let middle = resolve_at(&sketch, Point2::new(20.4, 10.6)).unwrap();
        assert_eq!(middle.target, Target::Midpoint(line));
        assert_eq!(middle.position, Point2::new(20.0, 10.0));
        assert_eq!(middle.target.entity(), Some(line));

        let only_points = resolve(
            &sketch,
            &Scaled(10.0),
            pointer_at(Point2::new(20.4, 10.6)),
            &[],
            Accept::Points,
            &[],
            &[],
        );
        assert_eq!(only_points, None);
        let on_circle = resolve(
            &sketch,
            &Scaled(10.0),
            pointer_at(Point2::new(20.4, 10.6)),
            &[],
            Accept::OnCircle {
                center: Point2::new(20.0, 10.0),
                radius: 50.0,
            },
            &[],
            &[],
        );
        assert_eq!(on_circle, None);
    }

    #[test]
    fn the_middle_of_an_arc_is_a_snap_target_however_far_it_sweeps() {
        let mut sketch = Sketch::new(Plane::XY);
        let quarter = sketch.add_arc(
            Point2::new(0.0, 0.0),
            Point2::new(10.0, 0.0),
            Point2::new(0.0, 10.0),
        );
        let most = sketch.add_arc(
            Point2::new(50.0, 0.0),
            Point2::new(60.0, 0.0),
            Point2::new(50.0, -10.0),
        );
        let diagonal = 10.0 / 2f64.sqrt();

        let middle = resolve_at(&sketch, Point2::new(diagonal + 0.3, diagonal - 0.2)).unwrap();
        let far_middle = resolve_at(&sketch, Point2::new(50.0 - diagonal, diagonal + 0.4)).unwrap();

        assert_eq!(middle.target, Target::Midpoint(quarter));
        assert!(middle.position.distance(Point2::new(diagonal, diagonal)) < 1e-9);
        assert_eq!(far_middle.target, Target::Midpoint(most));
        assert!(
            far_middle
                .position
                .distance(Point2::new(50.0 - diagonal, diagonal))
                < 1e-9
        );
    }

    fn outline(sketch: &mut Sketch, corners: &[Point2]) -> Vec<EntityId> {
        corners
            .iter()
            .zip(corners.iter().cycle().skip(1))
            .map(|(from, to)| sketch.add_line(*from, *to))
            .collect()
    }

    #[test]
    fn the_centre_of_a_closed_outline_symmetric_about_it_is_a_snap_target() {
        let mut sketch = Sketch::new(Plane::XY);
        let square = outline(
            &mut sketch,
            &[
                Point2::new(10.0, 10.0),
                Point2::new(30.0, 10.0),
                Point2::new(30.0, 30.0),
                Point2::new(10.0, 30.0),
            ],
        );
        let hexagon: Vec<Point2> = (0..6)
            .map(|step| {
                Point2::new(80.0, 20.0) + Vector2::from_angle(f64::from(step) * TAU / 6.0) * 10.0
            })
            .collect();
        outline(&mut sketch, &hexagon);
        outline(
            &mut sketch,
            &[
                Point2::new(10.0, -40.0),
                Point2::new(40.0, -40.0),
                Point2::new(30.0, -20.0),
                Point2::new(20.0, -20.0),
            ],
        );
        let corner = |line: EntityId, index: usize| sketch.entity(line).unwrap().points()[index];

        let centre = resolve_at(&sketch, Point2::new(20.4, 19.7)).unwrap();
        let hexagon_centre = resolve_at(&sketch, Point2::new(80.3, 20.2)).unwrap();
        let trapezoid_middle = resolve_at(&sketch, Point2::new(25.0, -30.0));

        assert_eq!(
            centre.target,
            Target::Centre {
                outline: square[0],
                corners: (corner(square[0], 1), corner(square[2], 1)),
            }
        );
        assert!(centre.position.distance(Point2::new(20.0, 20.0)) < 1e-9);
        assert!(matches!(hexagon_centre.target, Target::Centre { .. }));
        assert!(hexagon_centre.position.distance(Point2::new(80.0, 20.0)) < 1e-9);
        assert_eq!(trapezoid_middle, None);
    }

    #[test]
    fn an_open_chain_or_a_branching_corner_has_no_centre() {
        let mut sketch = Sketch::new(Plane::XY);
        let corners = [
            Point2::new(10.0, 10.0),
            Point2::new(30.0, 10.0),
            Point2::new(30.0, 30.0),
            Point2::new(10.0, 30.0),
        ];
        let open: Vec<EntityId> = corners
            .windows(2)
            .map(|pair| sketch.add_line(pair[0], pair[1]))
            .collect();
        assert_eq!(open.len(), 3);
        assert_eq!(resolve_at(&sketch, Point2::new(20.0, 20.0)), None);

        sketch.add_line(corners[3], corners[0]);
        sketch.add_line(corners[0], Point2::new(0.0, 0.0));
        assert_eq!(resolve_at(&sketch, Point2::new(20.0, 20.0)), None);
    }

    #[test]
    fn where_two_curves_cross_is_a_snap_target_ahead_of_either_curve() {
        let mut sketch = Sketch::new(Plane::XY);
        let across = sketch.add_line(Point2::new(10.0, 10.0), Point2::new(40.0, 40.0));
        let level = sketch.add_line(Point2::new(5.0, 20.0), Point2::new(55.0, 20.0));
        let circle = sketch.add_circle(Point2::new(60.0, 20.0), 10.0);

        let crossing = resolve_at(&sketch, Point2::new(20.4, 20.6)).unwrap();
        assert_eq!(crossing.target, Target::Intersection(across, level));
        assert!(crossing.position.distance(Point2::new(20.0, 20.0)) < 1e-9);
        assert_eq!(crossing.target.second_entity(), Some(level));

        let on_rim = resolve_at(&sketch, Point2::new(50.3, 20.4)).unwrap();
        assert_eq!(
            on_rim.target,
            Target::Intersection(level, circle).min_ordered()
        );
        assert!(on_rim.position.distance(Point2::new(50.0, 20.0)) < 1e-9);
    }

    #[test]
    fn curves_that_do_not_reach_each_other_have_no_crossing_to_snap_to() {
        let mut sketch = Sketch::new(Plane::XY);
        sketch.add_line(Point2::new(10.0, 10.0), Point2::new(18.0, 18.0));
        let level = sketch.add_line(Point2::new(5.0, 20.0), Point2::new(45.0, 20.0));

        let near_the_gap = resolve_at(&sketch, Point2::new(20.0, 20.3)).unwrap();

        assert_eq!(near_the_gap.target, Target::Curve(level));
    }

    #[test]
    fn the_axes_cross_other_curves_where_they_are_snapped() {
        let mut sketch = Sketch::new(Plane::XY);
        let line = sketch.add_line(Point2::new(10.0, -6.0), Point2::new(10.0, 14.0));

        let crossing = resolve_at(&sketch, Point2::new(10.3, 0.4)).unwrap();

        assert_eq!(
            crossing.target,
            Target::Intersection(
                EntityId::HORIZONTAL_AXIS.min(line),
                EntityId::HORIZONTAL_AXIS.max(line)
            )
        );
        assert!(crossing.position.distance(Point2::new(10.0, 0.0)) < 1e-9);
    }

    #[test]
    fn the_origin_and_the_axes_are_snap_targets() {
        let sketch = Sketch::new(Plane::XY);
        assert_eq!(
            resolve_at(&sketch, Point2::new(0.3, -0.4)).map(|snapped| snapped.target),
            Some(Target::Point(EntityId::ORIGIN))
        );
        let on_axis = resolve_at(&sketch, Point2::new(25.0, 0.4)).unwrap();
        assert_eq!(on_axis.target, Target::Curve(EntityId::HORIZONTAL_AXIS));
        assert_eq!(on_axis.position, Point2::new(25.0, 0.0));
        let on_axis = resolve_at(&sketch, Point2::new(-0.5, -12.0)).unwrap();
        assert_eq!(on_axis.target, Target::Curve(EntityId::VERTICAL_AXIS));
        assert_eq!(on_axis.position, Point2::new(0.0, -12.0));
    }

    #[test]
    fn curves_project_the_pointer_onto_themselves() {
        let mut sketch = Sketch::new(Plane::XY);
        let circle = sketch.add_circle(Point2::new(40.0, 40.0), 10.0);
        let arc = sketch.add_arc(
            Point2::new(-40.0, 40.0),
            Point2::new(-30.0, 40.0),
            Point2::new(-40.0, 50.0),
        );

        let rim = Point2::new(40.0, 40.0) + Vector2::from_angle(1.0) * 10.0;
        let on_circle = resolve_at(
            &sketch,
            Point2::new(40.0, 40.0) + Vector2::from_angle(1.0) * 9.6,
        )
        .unwrap();
        assert_eq!(on_circle.target, Target::Curve(circle));
        assert!(on_circle.position.distance(rim) < 1e-12);

        let off_middle = Point2::new(-40.0, 40.0) + Vector2::from_angle(0.2) * 10.3;
        let on_arc = resolve_at(&sketch, off_middle).unwrap();
        assert_eq!(on_arc.target, Target::Curve(arc));
        assert!((on_arc.position.distance(Point2::new(-40.0, 40.0)) - 10.0).abs() < 1e-12);

        assert_eq!(resolve_at(&sketch, Point2::new(-40.0, 29.7)), None);
    }

    #[test]
    fn pending_points_come_first_and_the_nearest_point_wins() {
        let mut sketch = Sketch::new(Plane::XY);
        let near = sketch.add_point(Point2::new(20.0, 0.3));
        sketch.add_point(Point2::new(20.6, 0.0));

        let snapped = resolve_at(&sketch, Point2::new(20.0, 0.2)).unwrap();
        assert_eq!(snapped.target, Target::Point(near));

        let pending = resolve(
            &sketch,
            &Scaled(10.0),
            pointer_at(Point2::new(20.0, 0.2)),
            &[(3, Point2::new(20.5, 0.5))],
            Accept::Anything,
            &[],
            &[],
        );
        assert_eq!(
            pending.map(|snapped| snapped.target),
            Some(Target::Pending(3))
        );
    }
    #[test]
    fn a_point_on_a_circle_snaps_where_a_spline_crosses_it() {
        let mut sketch = Sketch::new(Plane::XY);
        let spline = sketch.add_spline(&[
            Point2::new(20.0, 30.0),
            Point2::new(32.0, 52.0),
            Point2::new(60.0, 44.0),
        ]);
        let center = Point2::new(40.0, 40.0);
        let [crossing, ..] = sketch.circle_crossings(spline, center, 10.0)[..] else {
            panic!("the spline crosses the circle");
        };

        let snapped = resolve(
            &sketch,
            &Scaled(10.0),
            pointer_at(crossing + Vector2::new(0.2, 0.1)),
            &[],
            Accept::OnCircle {
                center,
                radius: 10.0,
            },
            &[],
            &[],
        )
        .unwrap();

        assert_eq!(snapped.target, Target::Curve(spline));
        assert!(snapped.position.distance(crossing) < 1e-9);
        assert!((snapped.position.distance(center) - 10.0).abs() < 1e-6);
    }

    #[test]
    fn a_point_on_a_circle_snaps_only_to_its_points_and_crossings() {
        let mut sketch = Sketch::new(Plane::XY);
        let on = sketch.add_point(Point2::new(40.0, 50.0));
        sketch.add_point(Point2::new(47.5, 47.0));
        let line = sketch.add_line(Point2::new(46.0, 20.0), Point2::new(46.0, 60.0));
        let arc = sketch.add_arc(
            Point2::new(60.0, 40.0),
            Point2::new(75.0, 40.0),
            Point2::new(45.0, 40.0),
        );
        let circle = Accept::OnCircle {
            center: Point2::new(40.0, 40.0),
            radius: 10.0,
        };
        let at = |point| {
            resolve(
                &sketch,
                &Scaled(10.0),
                pointer_at(point),
                &[],
                circle,
                &[],
                &[],
            )
        };
        let upper = Point2::new(46.875, 40.0 + (100.0f64 - 6.875 * 6.875).sqrt());
        let lower = Point2::new(upper.x, 80.0 - upper.y);

        let point = at(Point2::new(40.3, 50.2)).unwrap();
        let crossing = at(Point2::new(46.1, 48.3)).unwrap();
        let around = Accept::OnCircle {
            center: Point2::new(5.0, 3.0),
            radius: 5.0,
        };
        let axis_crossing = resolve(
            &sketch,
            &Scaled(10.0),
            pointer_at(Point2::new(9.2, 0.3)),
            &[],
            around,
            &[],
            &[],
        )
        .unwrap();

        assert_eq!(point.target, Target::Point(on));
        assert_eq!(at(Point2::new(47.5, 47.0)), None);
        assert_eq!(crossing.target, Target::Curve(line));
        assert!(crossing.position.distance(Point2::new(46.0, 48.0)) < 1e-12);
        assert_eq!(
            at(upper).map(|snapped| snapped.target),
            Some(Target::Curve(arc))
        );
        assert!(at(upper).unwrap().position.distance(upper) < 1e-9);
        assert_eq!(at(lower), None);
        assert_eq!(
            axis_crossing.target,
            Target::Curve(EntityId::HORIZONTAL_AXIS)
        );
        assert!(axis_crossing.position.distance(Point2::new(9.0, 0.0)) < 1e-12);
    }

    #[test]
    fn a_rim_snaps_to_points_but_not_to_curves() {
        let mut sketch = Sketch::new(Plane::XY);
        let point = sketch.add_point(Point2::new(30.0, 30.0));
        sketch.add_line(Point2::new(10.0, 10.0), Point2::new(10.0, 50.0));
        let rim = |at| {
            resolve(
                &sketch,
                &Scaled(10.0),
                pointer_at(at),
                &[],
                Accept::Points,
                &[],
                &[],
            )
        };

        assert_eq!(
            rim(Point2::new(30.2, 30.1)).map(|snapped| snapped.target),
            Some(Target::Point(point))
        );
        assert_eq!(rim(Point2::new(10.2, 30.0)), None);
    }

    #[test]
    fn a_ray_crosses_a_curve_nearest_the_pointer() {
        let mut sketch = Sketch::new(Plane::XY);
        let line = sketch.add_line(Point2::new(0.0, 10.0), Point2::new(20.0, 10.0));
        let circle = sketch.add_circle(Point2::new(50.0, 0.0), 10.0);
        let arc = sketch.add_arc(
            Point2::new(-50.0, 0.0),
            Point2::new(-40.0, 0.0),
            Point2::new(-60.0, 0.0),
        );
        let upward = |x: f64, curve, near| {
            crossing_along(
                &sketch,
                Target::Curve(curve),
                Point2::new(x, -30.0),
                Vector2::Y,
                near,
            )
        };

        assert_eq!(
            upward(5.0, line, Point2::new(5.0, 12.0)),
            Some(Point2::new(5.0, 10.0))
        );
        assert_eq!(upward(25.0, line, Point2::new(25.0, 10.0)), None);
        assert_eq!(
            upward(50.0, circle, Point2::new(50.0, -8.0)),
            Some(Point2::new(50.0, -10.0))
        );
        assert_eq!(
            upward(50.0, circle, Point2::new(50.0, 8.0)),
            Some(Point2::new(50.0, 10.0))
        );
        assert_eq!(
            upward(-50.0, arc, Point2::new(-50.0, -8.0)),
            Some(Point2::new(-50.0, 10.0))
        );
        assert_eq!(
            upward(7.0, EntityId::HORIZONTAL_AXIS, Point2::ZERO),
            Some(Point2::new(7.0, 0.0))
        );
        assert_eq!(upward(7.0, EntityId::VERTICAL_AXIS, Point2::ZERO), None);
    }

    #[test]
    fn an_acquired_line_extends_past_its_ends_for_snapping() {
        let mut sketch = Sketch::new(Plane::XY);
        let line = sketch.add_line(Point2::new(0.0, 0.0), Point2::new(10.0, 10.0));
        let beyond = Point2::new(30.2, 29.8);

        assert_eq!(resolve_at(&sketch, beyond), None);

        let extended = resolve(
            &sketch,
            &Scaled(10.0),
            pointer_at(beyond),
            &[],
            Accept::Anything,
            &[line],
            &[],
        )
        .unwrap();
        assert_eq!(extended.target, Target::Extension(line));
        assert!(extended.position.distance(Point2::new(30.0, 30.0)) < 1e-12);
        assert_eq!(
            extension_guide(&sketch, line, extended.position),
            Some([Point2::new(10.0, 10.0), extended.position])
        );

        let on_it = resolve(
            &sketch,
            &Scaled(10.0),
            pointer_at(Point2::new(3.2, 2.8)),
            &[],
            Accept::Anything,
            &[line],
            &[],
        )
        .unwrap();
        assert_eq!(on_it.target, Target::Curve(line));

        let across = crossing_along(
            &sketch,
            Target::Extension(line),
            Point2::new(20.0, 0.0),
            Vector2::Y,
            beyond,
        );
        assert_eq!(across, Some(Point2::new(20.0, 20.0)));
        let inside = crossing_along(
            &sketch,
            Target::Extension(line),
            Point2::new(5.0, 0.0),
            Vector2::Y,
            beyond,
        );
        assert_eq!(inside, None);
    }

    #[test]
    fn the_four_sides_of_a_circle_are_snap_targets_and_an_arc_keeps_those_it_sweeps() {
        let mut sketch = Sketch::new(Plane::XY);
        let circle = sketch.add_circle(Point2::new(50.0, 0.0), 10.0);
        let centre = point_of(&sketch, circle, 0);
        let arc_centre = Point2::new(-50.0, 30.0);
        sketch.add_arc(
            arc_centre,
            arc_centre + Vector2::from_angle(-PI / 4.0) * 10.0,
            arc_centre + Vector2::from_angle(3.0 * PI / 4.0) * 10.0,
        );

        let top = resolve_at(&sketch, Point2::new(50.4, 10.3)).unwrap();
        assert_eq!(
            top.target,
            Target::Quadrant {
                curve: circle,
                centre,
                side: Side::Top,
            }
        );
        assert_eq!(top.position, Point2::new(50.0, 10.0));
        assert!(matches!(
            resolve_at(&sketch, Point2::new(-50.3, 39.8)).map(|snapped| snapped.target),
            Some(Target::Quadrant {
                side: Side::Top,
                ..
            })
        ));
        assert_eq!(
            resolve_at(&sketch, Point2::new(-50.3, 20.2)).map(|snapped| snapped.target),
            None
        );
        assert!(matches!(
            resolve_at(&sketch, Point2::new(-39.8, 30.1)).map(|snapped| snapped.target),
            Some(Target::Quadrant {
                side: Side::Right,
                ..
            })
        ));
        assert_eq!(
            resolve_at(&sketch, Point2::new(-60.3, 30.1)).map(|snapped| snapped.target),
            None
        );
        assert_eq!(
            resolve(
                &sketch,
                &Scaled(10.0),
                pointer_at(Point2::new(50.4, 10.3)),
                &[],
                Accept::Points,
                &[],
                &[],
            ),
            None
        );
    }

    #[test]
    fn a_line_from_outside_a_circle_finds_where_it_would_touch_it() {
        let mut sketch = Sketch::new(Plane::XY);
        let circle = sketch.add_circle(Point2::ZERO, 10.0);
        let from = Point2::new(20.0, 0.0);
        let expected = Point2::new(5.0, 75.0_f64.sqrt());

        let touch = tangents_from(
            &sketch,
            &Scaled(10.0),
            pointer_at(expected + Vector2::new(0.3, 0.2)),
            from,
        )
        .unwrap();
        assert_eq!(touch.target, Target::Tangent(circle));
        assert!(touch.position.distance(expected) < 1e-12);
        assert!((touch.position - from).dot(touch.position).abs() < 1e-9);

        let inside = tangents_from(
            &sketch,
            &Scaled(10.0),
            pointer_at(expected),
            Point2::new(1.0, 0.0),
        );
        assert_eq!(inside, None);
        let far = tangents_from(
            &sketch,
            &Scaled(10.0),
            pointer_at(Point2::new(5.0, 6.0)),
            from,
        );
        assert_eq!(far, None);
    }

    #[test]
    fn a_slot_and_a_rounded_rectangle_have_centres_but_a_lopsided_one_does_not() {
        let mut sketch = Sketch::new(Plane::XY);
        let joined = |sketch: &mut Sketch, pieces: &[(Point2, Point2, Option<Point2>)]| {
            let ids: Vec<EntityId> = pieces
                .iter()
                .map(|(from, to, centre)| match centre {
                    Some(centre) => sketch.add_arc(*centre, *from, *to),
                    None => sketch.add_line(*from, *to),
                })
                .collect();
            ids
        };
        let slot = joined(
            &mut sketch,
            &[
                (Point2::new(0.0, -5.0), Point2::new(20.0, -5.0), None),
                (
                    Point2::new(20.0, -5.0),
                    Point2::new(20.0, 5.0),
                    Some(Point2::new(20.0, 0.0)),
                ),
                (Point2::new(20.0, 5.0), Point2::new(0.0, 5.0), None),
                (
                    Point2::new(0.0, 5.0),
                    Point2::new(0.0, -5.0),
                    Some(Point2::new(0.0, 0.0)),
                ),
            ],
        );

        let centre = resolve_at(&sketch, Point2::new(10.2, 0.3)).unwrap();
        assert!(matches!(centre.target, Target::Centre { outline, .. } if outline == slot[0]));
        assert!(centre.position.distance(Point2::new(10.0, 0.0)) < 1e-12);

        let mut lopsided = Sketch::new(Plane::XY);
        joined(
            &mut lopsided,
            &[
                (Point2::new(0.0, -5.0), Point2::new(20.0, -5.0), None),
                (
                    Point2::new(20.0, -5.0),
                    Point2::new(20.0, 5.0),
                    Some(Point2::new(20.0, 0.0)),
                ),
                (Point2::new(20.0, 5.0), Point2::new(0.0, 5.0), None),
                (Point2::new(0.0, 5.0), Point2::new(0.0, -5.0), None),
            ],
        );
        assert!(!matches!(
            resolve_at(&lopsided, Point2::new(10.2, 0.3)).map(|snapped| snapped.target),
            Some(Target::Centre { .. })
        ));
    }

    #[test]
    fn splines_take_points_and_cross_other_curves() {
        let mut sketch = Sketch::new(Plane::XY);
        let spline = sketch.add_spline(&[
            Point2::new(0.0, -20.0),
            Point2::new(10.0, 0.0),
            Point2::new(20.0, 20.0),
        ]);
        let line = sketch.add_line(Point2::new(-10.0, 5.0), Point2::new(30.0, 5.0));
        let on = sketch
            .closest_on_curve(spline, Point2::new(5.0, -9.0))
            .unwrap();

        let snapped = resolve_at(&sketch, on + Vector2::new(0.2, -0.2)).unwrap();
        assert_eq!(snapped.target, Target::Curve(spline));
        let resnapped = sketch.closest_on_curve(spline, snapped.position).unwrap();
        assert!(snapped.position.distance(resnapped) < 1e-9);
        assert!(snapped.position.distance(on) < 0.5);

        let crossing = sketch.spline_crossings(spline, line);
        let [crossing] = crossing[..] else {
            panic!("the line crosses the spline once");
        };
        let at = resolve_at(&sketch, crossing + Vector2::new(0.3, 0.2)).unwrap();
        assert_eq!(
            at.target.min_ordered(),
            Target::Intersection(spline, line).min_ordered()
        );
        assert!(at.position.distance(crossing) < 1e-9);

        let upward = crossing_along(
            &sketch,
            Target::Curve(spline),
            Point2::new(crossing.x, -40.0),
            Vector2::Y,
            crossing,
        )
        .unwrap();
        assert!(upward.distance(crossing) < 1e-6);
    }

    #[test]
    fn two_splines_snap_where_they_cross() {
        let mut sketch = Sketch::new(Plane::XY);
        let first = sketch.add_spline(&[
            Point2::new(0.0, -20.0),
            Point2::new(10.0, 0.0),
            Point2::new(20.0, 20.0),
        ]);
        let second = sketch.add_spline(&[
            Point2::new(0.0, 10.0),
            Point2::new(15.0, 20.0),
            Point2::new(30.0, 0.0),
        ]);
        let [crossing] = sketch.spline_crossings(first, second)[..] else {
            panic!("the splines cross once");
        };

        let at = resolve_at(&sketch, crossing + Vector2::new(0.3, -0.2)).unwrap();

        assert_eq!(
            at.target.min_ordered(),
            Target::Intersection(first, second).min_ordered()
        );
        assert!(at.position.distance(crossing) < 1e-9);
    }
}
