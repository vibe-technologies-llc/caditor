use std::{f64::consts::FRAC_1_SQRT_2, ops::RangeInclusive};

use ahash::AHashMap;
use caditor_geometry::{Point2, Vector2};

use crate::{ArcGeometry, Constraint, EntityId, Reference, Sketch};

const CONIC_SHOULDER: f64 = 0.5;
const LANE_TOLERANCE: f64 = 1e-6;
const LANE_DIRECTION_TOLERANCE: f64 = 1e-6;
const DEGENERATE: f64 = 1e-9;
pub const CIRCLE_LEADER_DIRECTION: Vector2 = Vector2::new(FRAC_1_SQRT_2, FRAC_1_SQRT_2);
const ARC_LEADER_FRACTION: f64 = 0.5;
const MAX_OBSTACLE_CELLS: i64 = 64;

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

    fn along(sketch: &Sketch, id: EntityId, other: EntityId) -> Option<Self> {
        let Some(arc) = sketch.arc(id) else {
            return Self::of(sketch, id);
        };
        let direction = sketch.angle_direction(id, other)?.try_normalize()?;
        Some(Self {
            origin: sketch.angle_vertex(id, other)?,
            direction,
            length: Some(arc.radius * arc.sweep.min(1.0)),
        })
    }

    pub fn origin(&self) -> Point2 {
        self.origin
    }

    pub fn direction(&self) -> Vector2 {
        self.direction
    }

    pub fn at(&self, parameter: f64) -> Point2 {
        self.origin + self.direction * parameter
    }

    pub fn parameter(&self, point: Point2) -> f64 {
        (point - self.origin).dot(self.direction)
    }

    pub fn foot(&self, point: Point2) -> Point2 {
        self.at(self.parameter(point))
    }

    pub fn clamped(&self, parameter: f64) -> f64 {
        self.length
            .map_or(parameter, |length| parameter.clamp(0.0, length))
    }

    pub fn closest(&self, point: Point2) -> Point2 {
        self.at(self.clamped(self.parameter(point)))
    }

    pub fn middle(&self) -> Option<Point2> {
        self.length.map(|length| self.at(length / 2.0))
    }

    pub fn ends(&self) -> Vec<Point2> {
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
        Constraint::AxisDiameter { point, axis, .. } => {
            let point = sketch.point(point)?;
            let mirrored = 2.0 * LineSpan::of(sketch, axis)?.foot(point) - point;
            Some(Measured::Points(point, mirrored))
        }
        Constraint::HorizontalDistance { .. } | Constraint::VerticalDistance { .. } => {
            let (from, to, along) = sketch.axis_offset_ends(constraint)?;
            Some(Measured::Aligned { from, to, along })
        }
        Constraint::Angle {
            from, to, reversed, ..
        } => Some(Measured::Angle(
            LineSpan::along(sketch, from, to)?,
            LineSpan::along(sketch, to, from)?,
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
        Constraint::MajorRadius { ellipse, .. } => {
            let shape = sketch.ellipse(ellipse)?;
            Some(Measured::Radius {
                center: shape.center,
                radius: shape.major_radius(),
                toward: shape.axis(),
            })
        }
        Constraint::MinorRadius { ellipse, .. } => {
            let shape = sketch.ellipse(ellipse)?;
            Some(Measured::Radius {
                center: shape.center,
                radius: shape.minor_radius,
                toward: shape.axis().perp(),
            })
        }
        Constraint::Rho { conic, .. } => {
            let (start, end) = sketch.entity(conic)?.spline_ends()?;
            let middle = sketch.point(start)?.lerp(sketch.point(end)?, 0.5);
            Some(Measured::Points(
                middle,
                sketch.spline(conic)?.point_at(CONIC_SHOULDER),
            ))
        }
        Constraint::Coincident(..)
        | Constraint::Horizontal(_)
        | Constraint::Vertical(_)
        | Constraint::HorizontalPoints(..)
        | Constraint::VerticalPoints(..)
        | Constraint::Parallel(..)
        | Constraint::Perpendicular(..)
        | Constraint::Tangent(..)
        | Constraint::Curvature(..)
        | Constraint::Equal(..)
        | Constraint::Midpoint { .. }
        | Constraint::OnMinorAxis { .. }
        | Constraint::Concentric(..)
        | Constraint::Collinear(..)
        | Constraint::Symmetric { .. }
        | Constraint::Fix { .. } => None,
    }
}

fn point_to_curve(sketch: &Sketch, point: Point2, curve: EntityId) -> Option<Measured> {
    if sketch.is_elliptic(curve) {
        return Some(Measured::Points(
            point,
            sketch.closest_on_ellipse(curve, point)?,
        ));
    }
    if let Some(line) = LineSpan::of(sketch, curve) {
        return Some(Measured::PointToLine(point, line));
    }
    if sketch.spline(curve).is_some() {
        return Some(Measured::Points(
            point,
            sketch.closest_on_curve(curve, point)?,
        ));
    }
    let (center, radius) = sketch.circle(curve)?;
    Some(Measured::PointToCircle {
        point,
        center,
        radius,
    })
}

fn curve_to_curve(sketch: &Sketch, from: EntityId, to: EntityId) -> Option<Measured> {
    let spline_gap = match (sketch.spline(from), sketch.spline(to)) {
        (Some(_), _) => Some(sketch.spline_gap(from, to)?),
        (None, Some(_)) => Some(sketch.spline_gap(to, from)?),
        (None, None) => None,
    };
    if let Some((on_spline, on_other)) = spline_gap {
        return Some(Measured::Points(on_spline, on_other));
    }
    let ellipse_gap = if sketch.is_elliptic(from) {
        sketch.ellipse_gap(from, to)
    } else if sketch.is_elliptic(to) {
        sketch.ellipse_gap(to, from)
    } else {
        None
    };
    if let Some((on_ellipse, on_other)) = ellipse_gap {
        return Some(Measured::Points(on_ellipse, on_other));
    }
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

pub fn away_from(normal: Vector2, at: Point2, centre: Option<Point2>) -> Vector2 {
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Footprint {
    pub center: Vector2,
    pub half: Vector2,
}

impl Footprint {
    pub fn area(&self) -> f64 {
        self.half.x * self.half.y * 4.0
    }

    pub fn overlap(&self, other: &Self) -> f64 {
        let reach = self.half + other.half;
        let apart = (self.center - other.center).abs();
        let narrower = self.half.min(other.half) * 2.0;
        let shared = (reach - apart).min(narrower).max(Vector2::ZERO);
        shared.x * shared.y
    }
}

#[derive(Debug, Clone, Default)]
struct Cell {
    footprints: Vec<usize>,
    covered: f64,
}

#[derive(Debug, Clone)]
pub struct Obstacles {
    cell: f64,
    footprints: Vec<(Footprint, (i64, i64))>,
    cells: AHashMap<(i64, i64), Cell>,
}

impl Obstacles {
    pub fn new(cell: f64) -> Self {
        Self {
            cell,
            footprints: Vec::new(),
            cells: AHashMap::new(),
        }
    }

    pub fn add(&mut self, footprint: Footprint) {
        let index = self.footprints.len();
        self.footprints
            .push((footprint, self.first_cell(&footprint)));
        for cell in self.cells_under(&footprint) {
            let covered = footprint.overlap(&self.cell_footprint(cell));
            let entry = self.cells.entry(cell).or_default();
            entry.footprints.push(index);
            entry.covered += covered;
        }
    }

    pub fn overlap(&self, footprint: &Footprint) -> f64 {
        let first = self.first_cell(footprint);
        let mut total = 0.0;
        for cell in self.cells_under(footprint) {
            let near = self
                .cells
                .get(&cell)
                .into_iter()
                .flat_map(|cell| &cell.footprints);
            for (other, other_first) in near.filter_map(|index| self.footprints.get(*index)) {
                let shared = (first.0.max(other_first.0), first.1.max(other_first.1));
                if shared == cell {
                    total += footprint.overlap(other);
                }
            }
        }
        total
    }

    pub fn covers_more_than(&self, footprint: &Footprint, share: f64) -> bool {
        self.overlap(footprint) > footprint.area() * share
    }

    pub fn cells_covered_over(&self, region: &Footprint, share: f64) -> bool {
        let full = self.cell * self.cell * share;
        self.cells_under(region).all(|cell| {
            self.cells
                .get(&cell)
                .is_some_and(|cell| cell.covered >= full)
        })
    }

    pub fn bounds(&self) -> Option<Footprint> {
        let mut corners = self.footprints.iter().flat_map(|(footprint, _)| {
            [
                footprint.center - footprint.half,
                footprint.center + footprint.half,
            ]
        });
        let first = corners.next()?;
        let (low, high) = corners.fold((first, first), |(low, high), corner| {
            (low.min(corner), high.max(corner))
        });
        Some(Footprint {
            center: (low + high) / 2.0,
            half: (high - low) / 2.0,
        })
    }

    fn cell_footprint(&self, (column, row): (i64, i64)) -> Footprint {
        let half = Vector2::splat(self.cell / 2.0);
        Footprint {
            center: Vector2::new(column as f64, row as f64) * self.cell + half,
            half,
        }
    }

    fn cell_span(&self, footprint: &Footprint) -> [RangeInclusive<i64>; 2] {
        let cell = |at: f64| (at / self.cell).floor() as i64;
        let low = footprint.center - footprint.half;
        let high = footprint.center + footprint.half;
        let span = |low: f64, high: f64| {
            let first = cell(low);
            first..=cell(high).min(first.saturating_add(MAX_OBSTACLE_CELLS))
        };
        [span(low.x, high.x), span(low.y, high.y)]
    }

    fn first_cell(&self, footprint: &Footprint) -> (i64, i64) {
        let [columns, rows] = self.cell_span(footprint);
        (*columns.start(), *rows.start())
    }

    fn cells_under(&self, footprint: &Footprint) -> impl Iterator<Item = (i64, i64)> + use<> {
        let [columns, rows] = self.cell_span(footprint);
        columns.flat_map(move |column| rows.clone().map(move |row| (column, row)))
    }
}

#[cfg(test)]
mod tests {
    use caditor_expression::Expression;
    use caditor_geometry::Plane;

    use super::*;

    fn assert_close(actual: Point2, expected: Point2) {
        assert!(
            actual.distance(expected) < 1e-9,
            "{actual} is not {expected}"
        );
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
                    value: Expression::Number(1.0),
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
    fn an_overall_distance_takes_the_lane_beyond_the_chain_it_spans() {
        let level = |from: f64, to: f64| Measured::Aligned {
            from: Point2::new(from, 0.0),
            to: Point2::new(to, 0.0),
            along: Vector2::X,
        };
        let measured = [
            Some(level(0.0, 40.0)),
            Some(level(0.0, 15.0)),
            Some(level(15.0, 40.0)),
            Some(level(0.0, 25.0)),
            None,
        ];

        let lanes = lanes(&measured, Some(Point2::new(20.0, 20.0)), 60.0);

        assert_eq!(lanes, vec![2, 0, 0, 1, 0]);
    }

    #[test]
    fn obstacles_in_any_unit_count_each_overlap_once_and_bound_what_they_hold() {
        let mut obstacles = Obstacles::new(2.0);
        let label = |x: f64, y: f64| Footprint {
            center: Vector2::new(x, y),
            half: Vector2::new(3.0, 1.0),
        };

        assert_eq!(obstacles.bounds(), None);

        obstacles.add(label(0.0, 0.0));
        obstacles.add(label(10.0, 4.0));

        assert_eq!(obstacles.overlap(&label(4.0, 0.5)), 2.0 * 1.5);
        assert_eq!(obstacles.overlap(&label(5.0, 2.0)), 0.0);
        assert_eq!(
            obstacles.bounds(),
            Some(Footprint {
                center: Vector2::new(5.0, 2.0),
                half: Vector2::new(8.0, 3.0),
            })
        );
    }
}
