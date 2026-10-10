use std::f64::consts::{PI, TAU};

use caditor_geometry::{Plane, Point2, Point3, Vector2, Vector3};
use thiserror::Error;

use crate::{
    bspline::BSpline,
    build::{
        SweepError,
        plan::{Plan, PlanCoedge, PlanFace, outward_sense},
        traversal_sense, traversal_tangent,
    },
    curve::{Curve, IntersectionCurve},
    error::GeometryError,
    interrupt::{self, Interrupted},
    interval::Interval,
    naming::{EdgeName, FaceName, FaceOrigin},
    profile::{Piece, ProfileError, Region},
    sense::Sense,
    surface::{BSplineSurface, Cone, Cylinder, Surface},
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
const SLOPE_STEP: f64 = 1e-4;

#[derive(Debug, Clone, PartialEq, Error)]
pub enum WrapError {
    #[error("no region is chosen to wrap")]
    NoRegions,
    #[error("only a cylinder or a cone unrolls flat, so nothing else can be wrapped onto")]
    NotUnrollable,
    #[error("the sketch plane is not parallel to the axis of the surface")]
    AcrossAxis,
    #[error("the curves reach past the apex of the cone")]
    PastApex,
    #[error("the outlines reach {turns} times round the surface")]
    BeyondFullTurn { turns: f64 },
    #[error("the curves are not one open chain")]
    NotOneChain,
    #[error("an end of the curve runs round the surface without leaving the faces")]
    EndRunsRound,
    #[error("the chosen faces have no common line along the axis to start the wrap from")]
    NoSeamPlace,
    #[error("the wrapped curve crosses itself")]
    CrossesItself,
    #[error("the wrapped curve closes round the surface")]
    ClosesRound,
    #[error("the wrapped curve does not cross the faces")]
    Misses,
    #[error("a wrapped curve needs more than {MAX_STATIONS} stations or turns")]
    TooIntricate,
    #[error("the curves could not be arranged: {0}")]
    Profile(ProfileError),
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

impl From<ProfileError> for WrapError {
    fn from(error: ProfileError) -> Self {
        match error {
            ProfileError::Cancelled(interrupted) => Self::Cancelled(interrupted),
            error => Self::Profile(error),
        }
    }
}

pub(super) struct Wrap<'a> {
    sketch: &'a Plane,
    centre: Point3,
    axis: Vector3,
    facing: Vector3,
    across: Vector3,
    radius: f64,
    sin: f64,
    cos: f64,
    reference: f64,
}

impl<'a> Wrap<'a> {
    pub(super) fn new(sketch: &'a Plane, surface: &Surface) -> Result<Self, WrapError> {
        let (frame, radius, half_angle) = match surface {
            Surface::Cylinder(cylinder) => (*cylinder.frame(), cylinder.radius(), 0.0),
            Surface::Cone(cone) => (*cone.frame(), cone.radius(), cone.half_angle()),
            _ => return Err(WrapError::NotUnrollable),
        };
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
        let (sin, cos) = f64::sin_cos(half_angle);
        Ok(Self {
            sketch,
            centre: frame.origin(),
            axis,
            facing,
            across: axis.cross(facing),
            radius,
            sin,
            cos,
            reference: 0.0,
        })
    }

    pub(super) fn referenced(mut self, points: &[Point2]) -> Result<Self, WrapError> {
        let (least, most) = points
            .iter()
            .map(|point| self.unrolled(*point).0)
            .fold(None, |found: Option<(f64, f64)>, along| match found {
                None => Some((along, along)),
                Some((least, most)) => Some((least.min(along), most.max(along))),
            })
            .ok_or(WrapError::NoRegions)?;
        self.reference = 0.5 * (least + most);
        Ok(self)
    }

    pub(super) fn unrolled(&self, point: Point2) -> (f64, f64) {
        let offset = self.sketch.to_world(point) - self.centre;
        (offset.dot(self.axis), offset.dot(self.across))
    }

    pub(super) fn sketch_point(&self, along: f64, round: f64) -> Point2 {
        self.sketch
            .to_local(self.centre + self.axis * along + self.across * round)
    }

    pub(super) fn across_in_sketch(&self) -> Vector2 {
        Vector2::new(
            self.across.dot(self.sketch.x_axis()),
            self.across.dot(self.sketch.y_axis()),
        )
    }

    pub(super) fn axis_in_sketch(&self) -> Vector2 {
        Vector2::new(
            self.axis.dot(self.sketch.x_axis()),
            self.axis.dot(self.sketch.y_axis()),
        )
    }

    fn slant(&self, along: f64) -> f64 {
        along - self.reference + self.reference / self.cos
    }

    pub(super) fn along_at_height(&self, height: f64) -> f64 {
        height / self.cos + self.reference - self.reference / self.cos
    }

    pub(super) fn radius_at(&self, along: f64) -> f64 {
        self.radius + self.slant(along) * self.sin
    }

    pub(super) fn radius_slope(&self) -> f64 {
        self.sin
    }

    pub(super) fn angle(&self, point: Point2) -> f64 {
        let (along, round) = self.unrolled(point);
        round / self.radius_at(along)
    }

    pub(super) fn angle_of(&self, point: Point3) -> (f64, f64) {
        let offset = point - self.centre;
        (
            offset.dot(self.axis),
            offset.dot(self.across).atan2(offset.dot(self.facing)),
        )
    }

    fn radial(&self, angle: f64) -> Vector3 {
        let (sin, cos) = angle.sin_cos();
        self.facing * cos + self.across * sin
    }

    fn point(&self, point: Point2, offset: f64) -> Point3 {
        let (along, round) = self.unrolled(point);
        let slant = self.slant(along);
        let local = self.radius + slant * self.sin;
        self.centre
            + self.axis * (slant * self.cos - offset * self.sin)
            + self.radial(round / local) * (local + offset * self.cos)
    }

    fn direction(&self, point: Point2, vector: Vector2, offset: f64) -> Vector3 {
        let step = SLOPE_STEP * self.radius_at(self.unrolled(point).0).max(1.0);
        let shift = vector * step;
        (self.point(point + shift, offset) - self.point(point - shift, offset)) / (2.0 * step)
    }

    fn up_is_outward(&self) -> bool {
        self.facing.dot(self.sketch.normal()) > 0.0
    }

    fn cap(&self, offset: f64, seam: f64) -> Result<Surface, WrapError> {
        let middle = self.reference / self.cos;
        let local = self.radius + middle * self.sin;
        let origin = self.centre + self.axis * (middle * self.cos - offset * self.sin);
        let frame = Plane::from_frame(origin, self.axis, self.radial(seam))
            .ok_or(WrapError::Geometry(GeometryError::ZeroDirection))?;
        let radius = local + offset * self.cos;
        Ok(if self.sin == 0.0 {
            Cylinder::new(frame, radius)?.into()
        } else {
            Cone::new(frame, radius, self.sin.atan2(self.cos))?.into()
        })
    }

    pub(super) fn band(&self, least_radius: f64) -> f64 {
        BAND * least_radius * self.cos
    }

    fn sampling(&self, least_radius: f64) -> Result<SamplingTolerance, WrapError> {
        SamplingTolerance::new(SAMPLING_CHORD * least_radius, SAMPLING_ANGLE)
            .ok_or(WrapError::Unassembled)
    }

    fn stations(&self, piece: &Piece, least_radius: f64) -> Result<Vec<f64>, WrapError> {
        let tolerance = self.sampling(least_radius)?;
        let curve = piece.curve();
        let seeds = curve.sample(piece.range(), &tolerance);
        let mut stations: Vec<f64> = Vec::with_capacity(seeds.len());
        for pair in seeds.windows(2) {
            let [from, to] = pair else {
                continue;
            };
            let turn = (self.angle(to.point) - self.angle(from.point)).abs();
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

    fn row(&self, piece: &Piece, stations: &[f64], offset: f64) -> Vec<Point3> {
        stations
            .iter()
            .map(|station| self.point(piece.curve().point(*station), offset))
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
        offset: f64,
    ) -> Result<IntersectionCurve, WrapError> {
        let points = self.row(piece, stations, offset);
        let [wall, cap] = surfaces;
        IntersectionCurve::through([wall.clone(), cap.clone()], &points, false)
            .ok_or(WrapError::Unassembled)
    }
}

struct Caps {
    offsets: (f64, f64),
    surfaces: (Surface, Surface),
    names: (FaceName, FaceName),
}

pub(super) struct Wrapping {
    pub(super) seam: f64,
    pub(super) least_radius: f64,
}

fn region_points(regions: &[Region]) -> Vec<Point2> {
    let extent = regions
        .iter()
        .filter_map(Region::bounds)
        .map(|bounds| bounds.size().length())
        .fold(0.0, f64::max);
    let tolerance = SamplingTolerance::for_extent(extent.max(LINEAR_RESOLUTION));
    regions
        .iter()
        .flat_map(Region::pieces)
        .flat_map(|piece| piece.curve().sample(piece.range(), &tolerance))
        .map(|sample| sample.point)
        .collect()
}

pub(super) fn least_radius(
    wrap: &Wrap<'_>,
    alongs: impl Iterator<Item = f64>,
) -> Result<f64, WrapError> {
    let least = alongs
        .map(|along| wrap.radius_at(along))
        .fold(f64::INFINITY, f64::min);
    if least.is_finite() && least > LINEAR_RESOLUTION {
        Ok(least)
    } else {
        Err(WrapError::PastApex)
    }
}

pub fn wrap_regions(
    plane: &Plane,
    regions: &[Region],
    surface: &Surface,
    feature: u64,
) -> Result<Solid, WrapError> {
    if regions.is_empty() {
        return Err(WrapError::NoRegions);
    }
    let points = region_points(regions);
    let wrap = Wrap::new(plane, surface)?.referenced(&points)?;
    let least = least_radius(&wrap, points.iter().map(|point| wrap.unrolled(*point).0))?;
    let (first, last) = points
        .iter()
        .map(|point| wrap.angle(*point))
        .fold(None, |found: Option<(f64, f64)>, angle| match found {
            None => Some((angle, angle)),
            Some((first, last)) => Some((first.min(angle), last.max(angle))),
        })
        .ok_or(WrapError::NoRegions)?;
    if last - first >= TAU - OVERLAP_GAP / least {
        return Err(WrapError::BeyondFullTurn {
            turns: (last - first) / TAU,
        });
    }
    let wrapping = Wrapping {
        seam: 0.5 * (first + last) + PI,
        least_radius: least,
    };
    wrapped(&wrap, regions, &wrapping, feature)
}

pub(super) fn wrapped(
    wrap: &Wrap<'_>,
    regions: &[Region],
    wrapping: &Wrapping,
    feature: u64,
) -> Result<Solid, WrapError> {
    let band = wrap.band(wrapping.least_radius);
    let (low, high) = if wrap.up_is_outward() {
        (-band, band)
    } else {
        (band, -band)
    };
    let cap_sense = |offset: f64| {
        if offset > 0.0 {
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
            offsets: (low, high),
            surfaces: (
                wrap.cap(low, wrapping.seam)?,
                wrap.cap(high, wrapping.seam)?,
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
            let (bottom_loop, top_loop) =
                wrap_loop(&mut plan, wrap, &pieces, &caps, wrapping, feature)?;
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
    wrapping: &Wrapping,
    feature: u64,
) -> Result<(Vec<PlanCoedge>, Vec<PlanCoedge>), WrapError> {
    let count = pieces.len();
    let (low, high) = caps.offsets;
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
        let stations = wrap.stations(piece, wrapping.least_radius)?;
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
