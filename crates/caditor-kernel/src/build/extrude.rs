use std::f64::consts::{PI, TAU};

use caditor_geometry::{Plane, Point2, Point3, Vector2, Vector3};

use crate::{
    build::{
        LinearBound, LinearExtent, SweepError, lift,
        plan::{Plan, PlanCoedge, PlanFace, outward_sense},
        traversal_sense, traversal_tangent,
    },
    curve::{Curve, Ellipse, Line},
    curve2::Curve2,
    error::GeometryError,
    interrupt,
    interval::Interval,
    naming::{EdgeName, FaceName, FaceOrigin},
    profile::{Piece, Region},
    sense::Sense,
    surface::{Cylinder, Extrusion, PlaneSurface, Surface},
    tolerance::{LINEAR_RESOLUTION, MAX_SIZE, PCURVE_TOLERANCE, SamplingTolerance},
    topology::{Pcurve, PcurveSample, Solid},
};

const ALONG_DIRECTION: f64 = 1e-6;
const FLAT: f64 = 0.5 * LINEAR_RESOLUTION;
const PCURVE_SAMPLING_ANGLE: f64 = 0.1;
const MAX_PCURVE_BISECTIONS: usize = 24;

fn offset_plane(plane: &Plane, offset: f64) -> Result<Plane, SweepError> {
    Plane::from_frame(
        plane.origin() + plane.normal() * offset,
        plane.normal(),
        plane.x_axis(),
    )
    .ok_or(SweepError::Geometry(GeometryError::ZeroDirection))
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Level {
    at_origin: f64,
    slope: Vector2,
}

impl Level {
    fn flat(height: f64) -> Self {
        Self {
            at_origin: height,
            slope: Vector2::ZERO,
        }
    }

    pub(super) fn of(sketch: &Plane, bound: LinearBound) -> Result<Self, SweepError> {
        match bound {
            LinearBound::Offset(height) => Ok(Self::flat(height)),
            LinearBound::Plane(target) => {
                let facing = target.normal().dot(sketch.normal());
                if facing.abs() <= ALONG_DIRECTION {
                    return Err(SweepError::EndAlongDirection);
                }
                let normal = target.normal() / facing;
                Ok(Self {
                    at_origin: normal.dot(target.origin() - sketch.origin()),
                    slope: -Vector2::new(normal.dot(sketch.x_axis()), normal.dot(sketch.y_axis())),
                })
            }
        }
    }

    pub(super) fn at(&self, point: Point2) -> f64 {
        self.at_origin + self.slope.dot(point)
    }

    fn is_flat(&self) -> bool {
        self.slope == Vector2::ZERO
    }

    fn minus(self, other: Self) -> Self {
        Self {
            at_origin: self.at_origin - other.at_origin,
            slope: self.slope - other.slope,
        }
    }
}

fn extreme_points(piece: &Piece, slope: Vector2) -> Vec<Point2> {
    let range = piece.range();
    let curve = piece.curve();
    let mut points = vec![curve.point(range.start()), curve.point(range.end())];
    match curve {
        Curve2::Line(_) => {}
        Curve2::Circle(circle) => {
            let along = Vector2::new(slope.dot(circle.x_axis()), slope.dot(circle.y_axis()));
            if along != Vector2::ZERO {
                let peak = along.y.atan2(along.x);
                for angle in [peak, peak + PI] {
                    let unwrapped = range.start() + (angle - range.start()).rem_euclid(TAU);
                    if unwrapped <= range.end() {
                        points.push(curve.point(unwrapped));
                    }
                }
            }
        }
        Curve2::BSpline(spline) => points.extend_from_slice(spline.control_points_over(range)),
    }
    points
}

pub(super) fn span(level: &Level, regions: &[Region]) -> Option<(f64, f64)> {
    regions
        .iter()
        .flat_map(Region::pieces)
        .flat_map(|piece| extreme_points(piece, level.slope))
        .map(|point| level.at(point))
        .fold(None, |found, height| match found {
            None => Some((height, height)),
            Some((least, most)) => Some((height.min(least), height.max(most))),
        })
}

fn settled(level: Level, regions: &[Region]) -> Level {
    if level.is_flat() {
        return level;
    }
    match span(&level, regions) {
        Some((least, most)) if most - least <= FLAT => Level::flat(0.5 * (least + most)),
        Some(_) | None => level,
    }
}

struct Cap {
    level: Level,
    plane: Plane,
}

impl Cap {
    fn new(sketch: &Plane, level: Level) -> Result<Self, SweepError> {
        let plane = if level.is_flat() {
            offset_plane(sketch, level.at_origin)?
        } else {
            let normal = sketch.normal() - lift(sketch, level.slope);
            let along_x = sketch.x_axis() + sketch.normal() * level.slope.x;
            Plane::with_x_axis(
                sketch.origin() + sketch.normal() * level.at_origin,
                normal,
                along_x,
            )
            .ok_or(SweepError::Geometry(GeometryError::ZeroDirection))?
        };
        Ok(Self { level, plane })
    }

    fn is_flat(&self) -> bool {
        self.level.is_flat()
    }

    fn point(&self, sketch: &Plane, point: Point2) -> Point3 {
        if self.is_flat() {
            self.plane.to_world(point)
        } else {
            sketch.to_world(point) + sketch.normal() * self.level.at(point)
        }
    }

    fn edge(&self, sketch: &Plane, piece: &Piece) -> Result<(Curve, Interval), SweepError> {
        let range = piece.range();
        if self.is_flat() {
            return Ok((piece.curve().on_plane(&self.plane)?, range));
        }
        let at = |point: Point2| self.point(sketch, point);
        Ok(match piece.curve() {
            Curve2::Line(line) => {
                let (from, to) = (at(line.point(range.start())), at(line.point(range.end())));
                let length =
                    Interval::new(0.0, from.distance(to)).ok_or(SweepError::Unassembled)?;
                (Line::through(from, to)?.into(), length)
            }
            Curve2::Circle(circle) => {
                let center = at(circle.center());
                let first = at(circle.center() + circle.x_axis() * circle.radius()) - center;
                let second = at(circle.center() + circle.y_axis() * circle.radius()) - center;
                let turn = 0.5
                    * (2.0 * first.dot(second))
                        .atan2(first.length_squared() - second.length_squared());
                let (sin, cos) = turn.sin_cos();
                let major = first * cos + second * sin;
                let minor = second * cos - first * sin;
                let frame = Plane::with_x_axis(center, major.cross(minor), major)
                    .ok_or(SweepError::Geometry(GeometryError::ZeroDirection))?;
                let ellipse = Ellipse::new(frame, major.length(), minor.length())?;
                let shifted = Interval::new(range.start() - turn, range.end() - turn)
                    .ok_or(SweepError::Unassembled)?;
                (ellipse.into(), shifted)
            }
            Curve2::BSpline(spline) => (Curve::BSpline(spline.map_points(at)?), range),
        })
    }
}

struct Caps {
    low: (FaceName, FaceOrigin),
    high: (FaceName, FaceOrigin),
}

struct Walls<'a> {
    sketch: &'a Plane,
    base: Plane,
    base_height: f64,
    low: Cap,
    high: Cap,
}

impl Walls<'_> {
    fn heights(&self, point: Point2) -> (f64, f64) {
        (
            self.low.level.at(point) - self.base_height,
            self.high.level.at(point) - self.base_height,
        )
    }

    fn mapped(&self) -> bool {
        self.low.is_flat() && self.high.is_flat()
    }
}

fn walls<'a>(
    sketch: &'a Plane,
    regions: &[Region],
    extent: LinearExtent,
) -> Result<(Walls<'a>, bool), SweepError> {
    let start = settled(Level::of(sketch, extent.start())?, regions);
    let end = settled(Level::of(sketch, extent.end())?, regions);
    for level in [start, end] {
        let (least, most) = span(&level, regions).ok_or(SweepError::NoRegions)?;
        if !least.is_finite() || !most.is_finite() {
            return Err(SweepError::NonFinite);
        }
        if least.abs().max(most.abs()) > MAX_SIZE {
            return Err(SweepError::TooLong);
        }
    }
    let (least, most) = span(&end.minus(start), regions).ok_or(SweepError::NoRegions)?;
    let start_is_low = if least > LINEAR_RESOLUTION {
        true
    } else if most < -LINEAR_RESOLUTION {
        false
    } else if start.is_flat() && end.is_flat() {
        return Err(SweepError::ZeroLength);
    } else {
        return Err(SweepError::EndsCross);
    };
    let (low, high) = if start_is_low {
        (start, end)
    } else {
        (end, start)
    };
    let (base_height, _) = span(&low, regions).ok_or(SweepError::NoRegions)?;
    Ok((
        Walls {
            sketch,
            base: offset_plane(sketch, base_height)?,
            base_height,
            low: Cap::new(sketch, low)?,
            high: Cap::new(sketch, high)?,
        },
        start_is_low,
    ))
}

pub fn extrude(
    plane: &Plane,
    regions: &[Region],
    extent: LinearExtent,
    feature: u64,
) -> Result<Solid, SweepError> {
    if regions.is_empty() {
        return Err(SweepError::NoRegions);
    }
    let (walls, start_is_low) = walls(plane, regions, extent)?;
    let mut plan = Plan::default();
    for region in regions {
        interrupt::check()?;
        plan.label(region.entities());
        let start = (
            FaceName::start_cap(feature, region.key()),
            FaceOrigin::StartCap { feature },
        );
        let end = (
            FaceName::end_cap(feature, region.key()),
            FaceOrigin::EndCap { feature },
        );
        let caps = if start_is_low {
            Caps {
                low: start,
                high: end,
            }
        } else {
            Caps {
                low: end,
                high: start,
            }
        };
        let mut bottom_loops = Vec::new();
        let mut top_loops = Vec::new();
        for profile_loop in region.loops() {
            let (bottom_loop, top_loop) =
                extrude_loop(&mut plan, &walls, profile_loop.pieces(), &caps, feature)?;
            bottom_loops.push(bottom_loop);
            top_loops.push(top_loop);
        }
        plan.face(PlanFace {
            surface: PlaneSurface::new(walls.low.plane.flipped())?.into(),
            sense: Sense::Same,
            name: caps.low.0,
            origin: Some(caps.low.1),
            loops: bottom_loops,
        });
        plan.face(PlanFace {
            surface: PlaneSurface::new(walls.high.plane)?.into(),
            sense: Sense::Same,
            name: caps.high.0,
            origin: Some(caps.high.1),
            loops: top_loops,
        });
    }
    Ok(plan.build()?)
}

fn extrude_loop(
    plan: &mut Plan,
    walls: &Walls<'_>,
    pieces: &[Piece],
    caps: &Caps,
    feature: u64,
) -> Result<(Vec<PlanCoedge>, Vec<PlanCoedge>), SweepError> {
    let sketch = walls.sketch;
    let count = pieces.len();
    let joints: Vec<Point2> = pieces.iter().map(Piece::start).collect();
    let lower: Vec<usize> = joints
        .iter()
        .map(|joint| plan.vertex(walls.low.point(sketch, *joint)))
        .collect();
    let upper: Vec<usize> = joints
        .iter()
        .map(|joint| plan.vertex(walls.high.point(sketch, *joint)))
        .collect();
    let sides: Vec<FaceName> = pieces
        .iter()
        .map(|piece| FaceName::side(feature, piece.id()))
        .collect();
    let at = |items: &[usize], index: usize| {
        items
            .get(index % count.max(1))
            .copied()
            .ok_or(SweepError::Unassembled)
    };
    let name_at = |index: usize| sides.get(index % count.max(1)).copied().unwrap_or_default();
    let mut verticals = Vec::with_capacity(count);
    for joint in 0..count {
        let before = name_at(joint + count - 1);
        let after = name_at(joint);
        verticals.push(plan.line(
            at(&lower, joint)?,
            at(&upper, joint)?,
            EdgeName::between(before, after),
        )?);
    }
    let mut bottom_loop = Vec::with_capacity(count);
    let mut top_loop = Vec::with_capacity(count);
    for (index, piece) in pieces.iter().enumerate() {
        let side = name_at(index);
        let travel = traversal_sense(piece);
        let ends = |vertices: &[usize]| -> Result<(usize, usize), SweepError> {
            let (from, to) = (at(vertices, index)?, at(vertices, index + 1)?);
            Ok(if piece.is_reversed() {
                (to, from)
            } else {
                (from, to)
            })
        };
        let (bottom_curve, bottom_range) = walls.low.edge(sketch, piece)?;
        let bottom_edge = plan.edge(
            bottom_curve,
            bottom_range,
            ends(&lower)?,
            EdgeName::between(side, caps.low.0),
        );
        let (top_curve, top_range) = walls.high.edge(sketch, piece)?;
        let top_edge = plan.edge(
            top_curve,
            top_range,
            ends(&upper)?,
            EdgeName::between(side, caps.high.0),
        );
        let (surface, mapped) = side_surface(&walls.base, piece)?;
        let middle = piece.range().middle();
        let middle_point = piece.curve().point(middle);
        let (low_middle, high_middle) = walls.heights(middle_point);
        let halfway = 0.5 * (low_middle + high_middle);
        let probe = walls.base.to_world(middle_point) + walls.base.normal() * halfway;
        let outward = lift(sketch, traversal_tangent(piece, middle)).cross(sketch.normal());
        let near = matches!(surface, Surface::Extrusion(_)).then(|| Point2::new(middle, halfway));
        let sense = outward_sense(&surface, probe, outward, near);
        let (entering, leaving) = (at(&verticals, index)?, at(&verticals, index + 1)?);
        let range = piece.range();
        let loop_coedges = if mapped {
            let (u_enter, u_leave) = (piece.start_parameter(), piece.end_parameter());
            let (bottom_enter, top_enter) = walls.heights(piece.start());
            let (bottom_leave, top_leave) = walls.heights(piece.end());
            let (bottom, top) = if walls.mapped() {
                (
                    PlanCoedge::mapped(
                        bottom_edge,
                        travel,
                        Point2::new(range.start(), bottom_enter),
                        Point2::new(range.end(), bottom_enter),
                    ),
                    PlanCoedge::mapped(
                        top_edge,
                        travel.reversed(),
                        Point2::new(range.start(), top_enter),
                        Point2::new(range.end(), top_enter),
                    ),
                )
            } else {
                (
                    PlanCoedge::given(
                        bottom_edge,
                        travel,
                        cap_pcurve(piece, &walls.low.level, walls.base_height, travel)?,
                    ),
                    PlanCoedge::given(
                        top_edge,
                        travel.reversed(),
                        cap_pcurve(
                            piece,
                            &walls.high.level,
                            walls.base_height,
                            travel.reversed(),
                        )?,
                    ),
                )
            };
            vec![
                bottom,
                PlanCoedge::mapped(
                    leaving,
                    Sense::Same,
                    Point2::new(u_leave, bottom_leave),
                    Point2::new(u_leave, top_leave),
                ),
                top,
                PlanCoedge::mapped(
                    entering,
                    Sense::Reversed,
                    Point2::new(u_enter, bottom_enter),
                    Point2::new(u_enter, top_enter),
                ),
            ]
        } else {
            vec![
                PlanCoedge::new(bottom_edge, travel),
                PlanCoedge::new(leaving, Sense::Same),
                PlanCoedge::new(top_edge, travel.reversed()),
                PlanCoedge::new(entering, Sense::Reversed),
            ]
        };
        plan.face(PlanFace {
            surface,
            sense,
            name: side,
            origin: Some(FaceOrigin::Side {
                feature,
                entity: piece.entity(),
            }),
            loops: vec![loop_coedges],
        });
        bottom_loop.push(PlanCoedge::new(bottom_edge, travel.reversed()));
        top_loop.push(PlanCoedge::new(top_edge, travel));
    }
    bottom_loop.reverse();
    Ok((bottom_loop, top_loop))
}

fn cap_pcurve(
    piece: &Piece,
    level: &Level,
    base_height: f64,
    sense: Sense,
) -> Result<Pcurve, SweepError> {
    let curve = piece.curve();
    let sample = |parameter: f64| PcurveSample {
        parameter,
        uv: Point2::new(parameter, level.at(curve.point(parameter)) - base_height),
    };
    let tolerance = SamplingTolerance::new(PCURVE_TOLERANCE, PCURVE_SAMPLING_ANGLE)
        .ok_or(SweepError::Unassembled)?;
    let seeds: Vec<PcurveSample> = curve
        .sample(piece.range(), &tolerance)
        .into_iter()
        .map(|seed| sample(seed.parameter))
        .collect();
    let mut samples: Vec<PcurveSample> = Vec::with_capacity(seeds.len());
    for pair in seeds.windows(2) {
        let [from, to] = pair else {
            continue;
        };
        let mut pending = vec![(*to, 0)];
        let mut last = *from;
        samples.push(last);
        while let Some((next, depth)) = pending.pop() {
            let middle = sample(0.5 * (last.parameter + next.parameter));
            let straight = 0.5 * (last.uv.y + next.uv.y);
            if depth < MAX_PCURVE_BISECTIONS
                && (middle.uv.y - straight).abs() > 0.25 * PCURVE_TOLERANCE
            {
                pending.push((next, depth + 1));
                pending.push((middle, depth + 1));
            } else {
                samples.push(next);
                last = next;
            }
        }
        samples.pop();
    }
    samples.extend(seeds.last().copied());
    if !sense.is_same() {
        samples.reverse();
    }
    Pcurve::new(samples, PCURVE_TOLERANCE).map_err(|_| SweepError::Unassembled)
}

fn side_surface(bottom: &Plane, piece: &Piece) -> Result<(Surface, bool), SweepError> {
    let normal: Vector3 = bottom.normal();
    Ok(match piece.curve() {
        Curve2::Line(_) => {
            let start = piece.start();
            let tangent = lift(bottom, traversal_tangent(piece, piece.start_parameter()));
            let frame = Plane::from_frame(bottom.to_world(start), tangent.cross(normal), tangent)
                .ok_or(SweepError::Geometry(GeometryError::ZeroDirection))?;
            (PlaneSurface::new(frame)?.into(), false)
        }
        Curve2::Circle(circle) => {
            let frame = Plane::from_frame(
                bottom.to_world(circle.center()),
                normal,
                lift(bottom, circle.x_axis()),
            )
            .ok_or(SweepError::Geometry(GeometryError::ZeroDirection))?;
            (Cylinder::new(frame, circle.radius())?.into(), false)
        }
        Curve2::BSpline(_) => (
            Extrusion::new(piece.curve().on_plane(bottom)?, normal)?.into(),
            true,
        ),
    })
}
