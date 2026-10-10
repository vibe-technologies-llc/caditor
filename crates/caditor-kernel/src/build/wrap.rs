use std::f64::consts::{PI, TAU};

use caditor_geometry::{Plane, Point2, Point3, Vector2, Vector3};
use thiserror::Error;

use crate::{
    bspline::BSpline,
    build::{
        SweepError, lift,
        plan::{Plan, PlanCoedge, PlanFace, outward_sense},
        traversal_sense, traversal_tangent,
    },
    curve::{Curve, IntersectionCurve},
    error::GeometryError,
    interrupt::{self, Interrupted},
    interval::Interval,
    naming::{EdgeName, FaceName, FaceOrigin},
    profile::{Piece, Region},
    sense::Sense,
    surface::{BSplineSurface, Cylinder, Surface},
    tolerance::{LINEAR_RESOLUTION, SamplingTolerance},
    topology::Solid,
};

const ACROSS_AXIS: f64 = 1e-6;
const BAND: f64 = 0.25;
const STEP_ANGLE: f64 = 0.05;
const SAMPLING_CHORD: f64 = 1e-3;
const SAMPLING_ANGLE: f64 = 0.1;
const WALL_DEGREE: usize = 3;
const WALL_OVERHANG: f64 = 0.25;
const MAX_STATIONS: usize = 4096;
const OVERLAP_GAP: f64 = 100.0 * LINEAR_RESOLUTION;

#[derive(Debug, Clone, PartialEq, Error)]
pub enum WrapError {
    #[error("no region is chosen to wrap")]
    NoRegions,
    #[error("the sketch plane is not parallel to the axis of the cylinder")]
    AcrossAxis,
    #[error("the outlines reach {span} mm round a cylinder {circumference} mm round")]
    BeyondFullTurn { span: f64, circumference: f64 },
    #[error("a wrapped curve needs more than {MAX_STATIONS} stations")]
    TooIntricate,
    #[error("the wrapped geometry cannot be built: {0}")]
    Geometry(#[from] GeometryError),
    #[error("the wrapped solid could not be assembled from the profile")]
    Unassembled,
    #[error("the wrapped solid is not valid: {0}")]
    Sweep(SweepError),
    #[error(transparent)]
    Cancelled(#[from] Interrupted),
}

impl From<SweepError> for WrapError {
    fn from(error: SweepError) -> Self {
        match error {
            SweepError::Cancelled(interrupted) => Self::Cancelled(interrupted),
            error => Self::Sweep(error),
        }
    }
}

struct Wrap<'a> {
    sketch: &'a Plane,
    centre: Point3,
    axis: Vector3,
    facing: Vector3,
    across: Vector3,
    radius: f64,
}

impl<'a> Wrap<'a> {
    fn new(sketch: &'a Plane, cylinder: &Cylinder) -> Result<Self, WrapError> {
        let frame = cylinder.frame();
        let axis = frame.normal();
        let normal = sketch.normal();
        if normal.dot(axis).abs() > ACROSS_AXIS {
            return Err(WrapError::AcrossAxis);
        }
        let offset = (sketch.origin() - frame.origin()).dot(normal);
        let facing = if offset < -LINEAR_RESOLUTION {
            -normal
        } else {
            normal
        };
        Ok(Self {
            sketch,
            centre: frame.origin(),
            axis,
            facing,
            across: axis.cross(facing),
            radius: cylinder.radius(),
        })
    }

    fn unrolled(&self, point: Point2) -> (f64, f64) {
        let offset = self.sketch.to_world(point) - self.centre;
        (offset.dot(self.axis), offset.dot(self.across))
    }

    fn radial(&self, angle: f64) -> Vector3 {
        let (sin, cos) = angle.sin_cos();
        self.facing * cos + self.across * sin
    }

    fn point(&self, point: Point2, radius: f64) -> Point3 {
        let (along, round) = self.unrolled(point);
        self.centre + self.axis * along + self.radial(round / self.radius) * radius
    }

    fn direction(&self, point: Point2, vector: Vector2, radius: f64) -> Vector3 {
        let lifted = lift(self.sketch, vector);
        let (_, round) = self.unrolled(point);
        let (sin, cos) = (round / self.radius).sin_cos();
        let tangential = self.across * cos - self.facing * sin;
        self.axis * lifted.dot(self.axis)
            + tangential * (lifted.dot(self.across) * radius / self.radius)
    }

    fn up_is_outward(&self) -> bool {
        self.facing.dot(self.sketch.normal()) > 0.0
    }

    fn cap(&self, radius: f64, middle: f64) -> Result<Cylinder, WrapError> {
        let frame = Plane::from_frame(
            self.centre,
            self.axis,
            self.radial(middle / self.radius + PI),
        )
        .ok_or(WrapError::Geometry(GeometryError::ZeroDirection))?;
        Ok(Cylinder::new(frame, radius)?)
    }

    fn stations(&self, piece: &Piece) -> Result<Vec<f64>, WrapError> {
        let tolerance = SamplingTolerance::new(SAMPLING_CHORD * self.radius, SAMPLING_ANGLE)
            .ok_or(WrapError::Unassembled)?;
        let curve = piece.curve();
        let seeds = curve.sample(piece.range(), &tolerance);
        let mut stations: Vec<f64> = Vec::with_capacity(seeds.len());
        for pair in seeds.windows(2) {
            let [from, to] = pair else {
                continue;
            };
            let (_, start) = self.unrolled(from.point);
            let (_, end) = self.unrolled(to.point);
            let turn = (end - start).abs() / self.radius;
            let steps = (turn / STEP_ANGLE).ceil().max(1.0);
            if stations.len() as f64 + steps > MAX_STATIONS as f64 {
                return Err(WrapError::TooIntricate);
            }
            let count = steps as usize;
            for step in 0..count {
                let share = step as f64 / steps;
                stations.push(from.parameter + (to.parameter - from.parameter) * share);
            }
        }
        stations.extend(seeds.last().map(|last| last.parameter));
        stations.dedup_by(|later, earlier| *later <= *earlier);
        if stations.len() < 2 {
            return Err(WrapError::Unassembled);
        }
        Ok(stations)
    }

    fn row(&self, piece: &Piece, stations: &[f64], radius: f64) -> Vec<Point3> {
        stations
            .iter()
            .map(|station| self.point(piece.curve().point(*station), radius))
            .collect()
    }

    fn wall(
        &self,
        piece: &Piece,
        stations: &[f64],
        (low, high): (f64, f64),
    ) -> Result<Surface, WrapError> {
        let degree = WALL_DEGREE.min(stations.len() - 1);
        let reach = (high - low) * WALL_OVERHANG;
        let bottom =
            BSpline::interpolating_at(degree, stations, &self.row(piece, stations, low - reach))?;
        let top =
            BSpline::interpolating_at(degree, stations, &self.row(piece, stations, high + reach))?;
        if bottom.knots() != top.knots() {
            return Err(WrapError::Unassembled);
        }
        let columns = bottom.control_points().len();
        let points: Vec<Point3> = bottom
            .control_points()
            .iter()
            .chain(top.control_points())
            .copied()
            .collect();
        Ok(BSplineSurface::new(
            degree,
            1,
            bottom.knots().to_vec(),
            vec![0.0, 0.0, 1.0, 1.0],
            columns,
            points,
            None,
        )?
        .into())
    }

    fn rim(
        &self,
        piece: &Piece,
        stations: &[f64],
        surfaces: [&Surface; 2],
        radius: f64,
    ) -> Result<IntersectionCurve, WrapError> {
        let points = self.row(piece, stations, radius);
        let [wall, cap] = surfaces;
        IntersectionCurve::through([wall.clone(), cap.clone()], &points, false)
            .ok_or(WrapError::Unassembled)
    }
}

struct Caps {
    radii: (f64, f64),
    surfaces: (Surface, Surface),
    names: (FaceName, FaceName),
}

fn round_span(wrap: &Wrap<'_>, regions: &[Region]) -> Result<(f64, f64), WrapError> {
    let tolerance = SamplingTolerance::new(SAMPLING_CHORD * wrap.radius, SAMPLING_ANGLE)
        .ok_or(WrapError::Unassembled)?;
    regions
        .iter()
        .flat_map(Region::pieces)
        .flat_map(|piece| piece.curve().sample(piece.range(), &tolerance))
        .map(|sample| wrap.unrolled(sample.point).1)
        .fold(None, |found: Option<(f64, f64)>, round| match found {
            None => Some((round, round)),
            Some((least, most)) => Some((least.min(round), most.max(round))),
        })
        .ok_or(WrapError::NoRegions)
}

pub fn wrap_regions(
    plane: &Plane,
    regions: &[Region],
    cylinder: &Cylinder,
    feature: u64,
) -> Result<Solid, WrapError> {
    if regions.is_empty() {
        return Err(WrapError::NoRegions);
    }
    let wrap = Wrap::new(plane, cylinder)?;
    let (least, most) = round_span(&wrap, regions)?;
    let circumference = TAU * wrap.radius;
    if most - least >= circumference - OVERLAP_GAP {
        return Err(WrapError::BeyondFullTurn {
            span: most - least,
            circumference,
        });
    }
    let middle = 0.5 * (least + most);
    let (inner, outer) = (wrap.radius * (1.0 - BAND), wrap.radius * (1.0 + BAND));
    let (low, high) = if wrap.up_is_outward() {
        (inner, outer)
    } else {
        (outer, inner)
    };
    let cap_sense = |radius: f64| {
        if radius > wrap.radius {
            Sense::Same
        } else {
            Sense::Reversed
        }
    };
    let mut plan = Plan::default();
    for region in regions {
        interrupt::check()?;
        plan.label(region.entities());
        let caps = Caps {
            radii: (low, high),
            surfaces: (
                wrap.cap(low, middle)?.into(),
                wrap.cap(high, middle)?.into(),
            ),
            names: (
                FaceName::start_cap(feature, region.key()),
                FaceName::end_cap(feature, region.key()),
            ),
        };
        let mut bottom_loops = Vec::new();
        let mut top_loops = Vec::new();
        for profile_loop in region.loops() {
            let pieces = opened(profile_loop.pieces())?;
            let (bottom_loop, top_loop) = wrap_loop(&mut plan, &wrap, &pieces, &caps, feature)?;
            bottom_loops.push(bottom_loop);
            top_loops.push(top_loop);
        }
        plan.face(PlanFace {
            surface: caps.surfaces.0,
            sense: cap_sense(low),
            name: caps.names.0,
            origin: Some(FaceOrigin::StartCap { feature }),
            loops: bottom_loops,
        });
        plan.face(PlanFace {
            surface: caps.surfaces.1,
            sense: cap_sense(high),
            name: caps.names.1,
            origin: Some(FaceOrigin::EndCap { feature }),
            loops: top_loops,
        });
    }
    plan.build().map_err(|error| SweepError::from(error).into())
}

fn opened(pieces: &[Piece]) -> Result<Vec<Piece>, WrapError> {
    let mut opened = Vec::with_capacity(pieces.len() + 1);
    for piece in pieces {
        if piece.start().distance(piece.end()) > LINEAR_RESOLUTION {
            opened.push(piece.clone());
            continue;
        }
        let range = piece.range();
        let middle = range.middle();
        let halves = [
            Interval::new(range.start(), middle),
            Interval::new(middle, range.end()),
        ]
        .map(|half| {
            half.map(|half| piece.with_curve(piece.curve().clone(), half))
                .ok_or(WrapError::Unassembled)
        });
        let [first, second] = halves;
        let (first, second) = (first?, second?);
        if piece.is_reversed() {
            opened.extend([second, first]);
        } else {
            opened.extend([first, second]);
        }
    }
    Ok(opened)
}

fn wrap_loop(
    plan: &mut Plan,
    wrap: &Wrap<'_>,
    pieces: &[Piece],
    caps: &Caps,
    feature: u64,
) -> Result<(Vec<PlanCoedge>, Vec<PlanCoedge>), WrapError> {
    let count = pieces.len();
    let (low, high) = caps.radii;
    let lower: Vec<usize> = pieces
        .iter()
        .map(|piece| plan.vertex(wrap.point(piece.start(), low)))
        .collect();
    let upper: Vec<usize> = pieces
        .iter()
        .map(|piece| plan.vertex(wrap.point(piece.start(), high)))
        .collect();
    let sides: Vec<FaceName> = pieces
        .iter()
        .map(|piece| FaceName::side(feature, piece.id()))
        .collect();
    let at = |items: &[usize], index: usize| {
        items
            .get(index % count.max(1))
            .copied()
            .ok_or(WrapError::Unassembled)
    };
    let name_at = |index: usize| sides.get(index % count.max(1)).copied().unwrap_or_default();
    let mut verticals = Vec::with_capacity(count);
    for joint in 0..count {
        verticals.push(plan.line(
            at(&lower, joint)?,
            at(&upper, joint)?,
            EdgeName::between(name_at(joint + count - 1), name_at(joint)),
        )?);
    }
    let mut bottom_loop = Vec::with_capacity(count);
    let mut top_loop = Vec::with_capacity(count);
    for (index, piece) in pieces.iter().enumerate() {
        interrupt::check()?;
        let side = name_at(index);
        let travel = traversal_sense(piece);
        let ends = |vertices: &[usize]| -> Result<(usize, usize), WrapError> {
            let (from, to) = (at(vertices, index)?, at(vertices, index + 1)?);
            Ok(if piece.is_reversed() {
                (to, from)
            } else {
                (from, to)
            })
        };
        let stations = wrap.stations(piece)?;
        let wall = wrap.wall(piece, &stations, (low, high))?;
        let bottom_curve = wrap.rim(piece, &stations, [&wall, &caps.surfaces.0], low)?;
        let bottom_range = bottom_curve.domain();
        let bottom_edge = plan.edge(
            Curve::Intersection(bottom_curve),
            bottom_range,
            ends(&lower)?,
            EdgeName::between(side, caps.names.0),
        );
        let top_curve = wrap.rim(piece, &stations, [&wall, &caps.surfaces.1], high)?;
        let top_range = top_curve.domain();
        let top_edge = plan.edge(
            Curve::Intersection(top_curve),
            top_range,
            ends(&upper)?,
            EdgeName::between(side, caps.names.1),
        );
        let middle = piece.range().middle();
        let middle_point = piece.curve().point(middle);
        let halfway = 0.5 * (low + high);
        let probe = wrap.point(middle_point, halfway);
        let outward_in_sketch = traversal_tangent(piece, middle).perp() * -1.0;
        let outward = wrap.direction(middle_point, outward_in_sketch, halfway);
        let near = Point2::new(middle, 0.5);
        let sense = outward_sense(&wall, probe, outward, Some(near));
        let (entering, leaving) = (at(&verticals, index)?, at(&verticals, index + 1)?);
        plan.face(PlanFace {
            surface: wall,
            sense,
            name: side,
            origin: Some(FaceOrigin::Side {
                feature,
                entity: piece.entity(),
            }),
            loops: vec![vec![
                PlanCoedge::new(bottom_edge, travel),
                PlanCoedge::new(leaving, Sense::Same),
                PlanCoedge::new(top_edge, travel.reversed()),
                PlanCoedge::new(entering, Sense::Reversed),
            ]],
        });
        bottom_loop.push(PlanCoedge::new(bottom_edge, travel.reversed()));
        top_loop.push(PlanCoedge::new(top_edge, travel));
    }
    bottom_loop.reverse();
    Ok((bottom_loop, top_loop))
}
