use std::{
    collections::BTreeSet,
    f64::consts::{FRAC_PI_2, TAU},
};

use caditor_geometry::{Aabb2, Plane, Point2, Point3, RigidTransform, Vector2, Vector3};

use crate::{
    bspline::BSpline,
    build::{
        AngularExtent, Axis2, SweepError, lift,
        plan::{Plan, PlanCoedge, PlanFace, outward_sense},
        traversal_sense, traversal_tangent,
    },
    curve::Circle,
    curve2::{BSplineCurve2, Circle2, Curve2, Ellipse2, Line2},
    error::GeometryError,
    interrupt,
    interval::Interval,
    naming::{EdgeName, FaceName, FaceOrigin},
    profile::{Piece, Region},
    sense::Sense,
    surface::{Cone, Cylinder, PlaneSurface, Revolution, Sphere, Surface, Torus},
    tolerance::{LINEAR_RESOLUTION, SamplingTolerance},
    topology::Solid,
};

const RELATIVE_TOLERANCE: f64 = 1e-7;
const FLAT: f64 = 0.5 * LINEAR_RESOLUTION;
const SENSE_PROBES: u32 = 8;

fn degenerate() -> SweepError {
    SweepError::Geometry(GeometryError::ZeroDirection)
}

struct Frame {
    plane: Plane,
    axis: Axis2,
    origin: Point3,
    direction: Vector3,
    low: RigidTransform,
    high: RigidTransform,
    sweep: f64,
    full: bool,
    tolerance: f64,
}

impl Frame {
    fn radial(&self) -> Vector3 {
        self.low
            .apply_vector(lift(&self.plane, self.axis.direction().perp()))
    }

    fn on_axis(&self, point: Point2) -> Point3 {
        self.origin + self.direction * self.axis.along(point)
    }

    fn meridian(&self, height: f64) -> Result<Plane, SweepError> {
        Plane::from_frame(
            self.origin + self.direction * height,
            self.direction,
            self.radial(),
        )
        .ok_or_else(degenerate)
    }

    fn at_low(&self, point: Point2) -> Point3 {
        self.low.apply_point(self.plane.to_world(point))
    }

    fn at_high(&self, point: Point2) -> Point3 {
        self.high.apply_point(self.plane.to_world(point))
    }

    fn touches_axis(&self, point: Point2) -> bool {
        self.axis.signed_distance(point).abs() <= self.tolerance
    }
}

struct Caps {
    low: (FaceName, FaceOrigin),
    high: (FaceName, FaceOrigin),
}

pub fn revolve(
    plane: &Plane,
    regions: &[Region],
    axis: Axis2,
    extent: AngularExtent,
    feature: u64,
) -> Result<Solid, SweepError> {
    if regions.is_empty() {
        return Err(SweepError::NoRegions);
    }
    let size = regions
        .iter()
        .filter_map(Region::bounds)
        .reduce(Aabb2::union)
        .map_or(0.0, |bounds| bounds.size().length());
    let tolerance = (size * RELATIVE_TOLERANCE).max(LINEAR_RESOLUTION);
    let on_right = side_of_axis(regions, &axis, size, tolerance)?;
    let (axis, start, end) = if on_right {
        (axis.reversed(), -extent.start(), -extent.end())
    } else {
        (axis, extent.start(), extent.end())
    };
    let (low, high) = (start.min(end), start.max(end));
    let origin = plane.to_world(axis.origin());
    let direction = lift(plane, axis.direction());
    let turn = |angle: f64| {
        RigidTransform::rotation_about(origin, direction, angle).ok_or_else(degenerate)
    };
    let full = extent.is_full();
    let frame = Frame {
        plane: *plane,
        axis,
        origin,
        direction,
        low: turn(low)?,
        high: turn(if full { low } else { high })?,
        sweep: if full { TAU } else { high - low },
        full,
        tolerance,
    };
    let mut plan = Plan::default();
    for region in regions {
        interrupt::check()?;
        plan.label(region.entities());
        let start_cap = (
            FaceName::start_cap(feature, region.key()),
            FaceOrigin::StartCap { feature },
        );
        let end_cap = (
            FaceName::end_cap(feature, region.key()),
            FaceOrigin::EndCap { feature },
        );
        let caps = if start <= end {
            Caps {
                low: start_cap,
                high: end_cap,
            }
        } else {
            Caps {
                low: end_cap,
                high: start_cap,
            }
        };
        let mut low_loops = Vec::new();
        let mut high_loops = Vec::new();
        for profile_loop in region.loops() {
            let (low_loop, high_loop) =
                revolve_loop(&mut plan, &frame, profile_loop.pieces(), &caps, feature)?;
            low_loops.push(low_loop);
            high_loops.push(high_loop);
        }
        if !full {
            let normal = plane.normal();
            let x_axis = plane.x_axis();
            let low_plane = Plane::from_frame(
                origin,
                -frame.low.apply_vector(normal),
                frame.low.apply_vector(x_axis),
            )
            .ok_or_else(degenerate)?;
            let high_plane = Plane::from_frame(
                origin,
                frame.high.apply_vector(normal),
                frame.high.apply_vector(x_axis),
            )
            .ok_or_else(degenerate)?;
            plan.face(PlanFace {
                surface: PlaneSurface::new(low_plane)?.into(),
                sense: Sense::Same,
                name: caps.low.0,
                origin: Some(caps.low.1),
                loops: low_loops,
            });
            plan.face(PlanFace {
                surface: PlaneSurface::new(high_plane)?.into(),
                sense: Sense::Same,
                name: caps.high.0,
                origin: Some(caps.high.1),
                loops: high_loops,
            });
        }
    }
    Ok(plan.build()?)
}

fn side_of_axis(
    regions: &[Region],
    axis: &Axis2,
    size: f64,
    tolerance: f64,
) -> Result<bool, SweepError> {
    let sampling = SamplingTolerance::for_extent(size);
    let (mut left, mut right, mut crossing) = (BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
    for piece in regions.iter().flat_map(Region::pieces) {
        let distances: Vec<f64> = piece
            .curve()
            .sample(piece.range(), &sampling)
            .iter()
            .map(|sample| axis.signed_distance(sample.point))
            .collect();
        let most = distances.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let least = distances.iter().copied().fold(f64::INFINITY, f64::min);
        match (most > tolerance, least < -tolerance) {
            (true, true) => {
                crossing.insert(piece.entity());
            }
            (true, false) => {
                left.insert(piece.entity());
            }
            (false, true) => {
                right.insert(piece.entity());
            }
            (false, false) => {}
        }
    }
    if !crossing.is_empty() {
        return Err(SweepError::CrossesAxis {
            entities: crossing.into_iter().collect(),
        });
    }
    match (left.is_empty(), right.is_empty()) {
        (true, true) => Err(SweepError::OnAxis),
        (false, false) => Err(SweepError::BothSidesOfAxis {
            left: left.into_iter().collect(),
            right: right.into_iter().collect(),
        }),
        (false, true) => Ok(false),
        (true, false) => Ok(true),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    OnAxis,
    Flat,
    Curved { mapped: bool },
}

struct Prepared {
    piece: Piece,
    kind: Kind,
}

fn prepare(frame: &Frame, piece: &Piece) -> Result<Prepared, SweepError> {
    let axis = &frame.axis;
    match piece.curve() {
        Curve2::Line(line) => {
            let range = piece.range();
            let snap = |point: Point2| {
                let distance = axis.signed_distance(point);
                if distance.abs() <= frame.tolerance {
                    point - axis.direction().perp() * distance
                } else {
                    point
                }
            };
            let (from, to) = (
                snap(line.point(range.start())),
                snap(line.point(range.end())),
            );
            let rebuilt = Line2::through(from, to)?;
            let length = Interval::new(0.0, from.distance(to)).ok_or_else(degenerate)?;
            let kind = if frame.touches_axis(from) && frame.touches_axis(to) {
                Kind::OnAxis
            } else if (axis.along(to) - axis.along(from)).abs() <= FLAT {
                Kind::Flat
            } else {
                Kind::Curved { mapped: false }
            };
            Ok(Prepared {
                piece: piece.with_curve(rebuilt.into(), length),
                kind,
            })
        }
        Curve2::Circle(circle) => {
            let center_distance = axis.signed_distance(circle.center());
            if center_distance.abs() <= frame.tolerance
                || (center_distance > frame.tolerance
                    && nearest_to_axis(frame, circle, piece.range()) > frame.tolerance)
            {
                Ok(Prepared {
                    piece: piece.clone(),
                    kind: Kind::Curved { mapped: false },
                })
            } else {
                let spline = arc_spline(circle, piece.range())?;
                let domain = spline.domain();
                Ok(Prepared {
                    piece: piece.with_curve(spline.into(), domain),
                    kind: Kind::Curved { mapped: true },
                })
            }
        }
        Curve2::BSpline(_) => Ok(Prepared {
            piece: piece.clone(),
            kind: Kind::Curved { mapped: true },
        }),
        Curve2::Ellipse(ellipse) => {
            let spline = ellipse_spline(ellipse, piece.range())?;
            let domain = spline.domain();
            Ok(Prepared {
                piece: piece.with_curve(spline.into(), domain),
                kind: Kind::Curved { mapped: true },
            })
        }
    }
}

fn nearest_to_axis(frame: &Frame, circle: &Circle2, range: Interval) -> f64 {
    let toward_axis = circle.center() - frame.axis.direction().perp() * circle.radius();
    let [nearest, _, _] = circle.evaluate(circle.closest_parameter(toward_axis, range));
    frame.axis.signed_distance(nearest)
}

pub(super) fn ellipse_spline(
    ellipse: &Ellipse2,
    range: Interval,
) -> Result<BSplineCurve2, GeometryError> {
    let unit = Circle2::new(Point2::ZERO, 1.0)?;
    let center = ellipse.center();
    let (along_x, along_y) = (
        ellipse.x_axis() * ellipse.major_radius(),
        ellipse.y_axis() * ellipse.minor_radius(),
    );
    arc_spline(&unit, range)?.map_points(|point| center + along_x * point.x + along_y * point.y)
}

pub(super) fn arc_spline(
    circle: &Circle2,
    range: Interval,
) -> Result<BSplineCurve2, GeometryError> {
    let curve = Curve2::from(*circle);
    let pieces = (range.length() / FRAC_PI_2).ceil().max(1.0) as usize;
    let step = range.length() / pieces as f64;
    let weight = (0.5 * step).cos();
    let mut points = Vec::with_capacity(2 * pieces + 1);
    let mut weights = Vec::with_capacity(2 * pieces + 1);
    let mut knots = vec![0.0; 3];
    for index in 0..pieces {
        let angle = range.start() + step * index as f64;
        points.push(curve.point(angle));
        weights.push(1.0);
        let middle = curve.point(angle + 0.5 * step);
        points.push(circle.center() + (middle - circle.center()) / weight);
        weights.push(weight);
        if index > 0 {
            let knot = index as f64 / pieces as f64;
            knots.extend([knot, knot]);
        }
    }
    points.push(curve.point(range.end()));
    weights.push(1.0);
    knots.extend([1.0; 3]);
    BSpline::rational(2, knots, points, weights)
}

fn surface_of(frame: &Frame, prepared: &Prepared) -> Result<Surface, SweepError> {
    let axis = &frame.axis;
    let piece = &prepared.piece;
    Ok(match (piece.curve(), prepared.kind) {
        (_, Kind::Curved { mapped: true }) => {
            let profile = piece
                .curve()
                .on_plane(&frame.plane)?
                .transformed(&frame.low)?;
            Revolution::new(profile, frame.origin, frame.direction)?.into()
        }
        (Curve2::Line(_), kind) => {
            let (from, to) = (piece.start(), piece.end());
            let (from, to) = if axis.along(from) <= axis.along(to) {
                (from, to)
            } else {
                (to, from)
            };
            let (low_radius, high_radius) = (axis.signed_distance(from), axis.signed_distance(to));
            let (low_height, high_height) = (axis.along(from), axis.along(to));
            if kind == Kind::Flat {
                PlaneSurface::new(frame.meridian(0.5 * (low_height + high_height))?)?.into()
            } else if (high_radius - low_radius).abs() <= FLAT {
                Cylinder::new(
                    frame.meridian(low_height)?,
                    0.5 * (low_radius + high_radius),
                )?
                .into()
            } else {
                let half_angle = (high_radius - low_radius).atan2(high_height - low_height);
                Cone::new(frame.meridian(low_height)?, low_radius.max(0.0), half_angle)?.into()
            }
        }
        (Curve2::Circle(circle), _) => {
            let center_distance = axis.signed_distance(circle.center());
            let height = axis.along(circle.center());
            if center_distance.abs() <= frame.tolerance {
                Sphere::new(frame.meridian(height)?, circle.radius())?.into()
            } else {
                Torus::new(frame.meridian(height)?, center_distance, circle.radius())?.into()
            }
        }
        (Curve2::BSpline(_) | Curve2::Ellipse(_), _) => {
            let profile = piece
                .curve()
                .on_plane(&frame.plane)?
                .transformed(&frame.low)?;
            Revolution::new(profile, frame.origin, frame.direction)?.into()
        }
    })
}

fn farthest_from_axis(frame: &Frame, piece: &Piece) -> f64 {
    let range = piece.range();
    let reach = |parameter: f64| {
        frame
            .axis
            .signed_distance(piece.curve().point(parameter))
            .abs()
    };
    (1..SENSE_PROBES)
        .map(|probe| range.start() + range.length() * f64::from(probe) / f64::from(SENSE_PROBES))
        .fold(
            (range.middle(), reach(range.middle())),
            |best, parameter| {
                let distance = reach(parameter);
                if distance > best.1 {
                    (parameter, distance)
                } else {
                    best
                }
            },
        )
        .0
}

fn revolve_loop(
    plan: &mut Plan,
    frame: &Frame,
    pieces: &[Piece],
    caps: &Caps,
    feature: u64,
) -> Result<(Vec<PlanCoedge>, Vec<PlanCoedge>), SweepError> {
    let prepared = pieces
        .iter()
        .map(|piece| prepare(frame, piece))
        .collect::<Result<Vec<_>, _>>()?;
    let count = prepared.len();
    let wrap = |index: usize| index % count.max(1);
    let joints: Vec<Point2> = prepared.iter().map(|entry| entry.piece.start()).collect();
    let poles: Vec<bool> = joints
        .iter()
        .map(|joint| frame.touches_axis(*joint))
        .collect();
    let mut low_vertices = Vec::with_capacity(count);
    let mut high_vertices = Vec::with_capacity(count);
    for (joint, pole) in joints.iter().zip(&poles) {
        if *pole {
            let vertex = plan.vertex(frame.on_axis(*joint));
            low_vertices.push(vertex);
            high_vertices.push(vertex);
        } else {
            let vertex = plan.vertex(frame.at_low(*joint));
            low_vertices.push(vertex);
            high_vertices.push(if frame.full {
                vertex
            } else {
                plan.vertex(frame.at_high(*joint))
            });
        }
    }
    let at = |items: &[usize], index: usize| {
        items
            .get(wrap(index))
            .copied()
            .ok_or(SweepError::Unassembled)
    };
    let sides: Vec<FaceName> = prepared
        .iter()
        .map(|entry| FaceName::side(feature, entry.piece.id()))
        .collect();
    let side = |index: usize| sides.get(wrap(index)).copied().unwrap_or_default();
    let mut swept: Vec<Option<usize>> = Vec::with_capacity(count);
    for (index, (joint, pole)) in joints.iter().zip(&poles).enumerate() {
        if *pole {
            swept.push(None);
            continue;
        }
        let center = frame.on_axis(*joint);
        let radius = frame.axis.signed_distance(*joint);
        let start = frame.at_low(*joint);
        let circle_frame =
            Plane::from_frame(center, frame.direction, start - center).ok_or_else(degenerate)?;
        let circle = Circle::new(circle_frame, radius)?;
        let interval = Interval::new(0.0, frame.sweep).ok_or_else(degenerate)?;
        swept.push(Some(plan.edge(
            circle.into(),
            interval,
            (at(&low_vertices, index)?, at(&high_vertices, index)?),
            EdgeName::between(side(index + count - 1), side(index)),
        )));
    }
    let swept_at = |index: usize| swept.get(wrap(index)).copied().flatten();
    let mut low_loop = Vec::with_capacity(count);
    let mut high_loop = Vec::with_capacity(count);
    for (index, entry) in prepared.iter().enumerate() {
        let piece = &entry.piece;
        let travel = traversal_sense(piece);
        let ends = |vertices: &[usize]| -> Result<(usize, usize), SweepError> {
            let (from, to) = (at(vertices, index)?, at(vertices, index + 1)?);
            Ok(if piece.is_reversed() {
                (to, from)
            } else {
                (from, to)
            })
        };
        let lifted = piece.curve().on_plane(&frame.plane)?;
        if entry.kind == Kind::OnAxis {
            if !frame.full {
                let edge = plan.edge(
                    lifted.transformed(&frame.low)?,
                    piece.range(),
                    ends(&low_vertices)?,
                    EdgeName::between(caps.low.0, caps.high.0),
                );
                low_loop.push(PlanCoedge::new(edge, travel.reversed()));
                high_loop.push(PlanCoedge::new(edge, travel));
            }
            continue;
        }
        let name = side(index);
        let surface = surface_of(frame, entry)?;
        let probe = farthest_from_axis(frame, piece);
        let tangent = traversal_tangent(piece, probe);
        let outward = frame
            .low
            .apply_vector(lift(&frame.plane, Vector2::new(tangent.y, -tangent.x)));
        let near = (entry.kind == Kind::Curved { mapped: true }).then(|| Point2::new(0.0, probe));
        let sense = outward_sense(
            &surface,
            frame.at_low(piece.curve().point(probe)),
            outward,
            near,
        );
        let origin = FaceOrigin::Side {
            feature,
            entity: piece.entity(),
        };
        if frame.full && entry.kind == Kind::Flat {
            let loops = flat_disc_loops(frame, &joints, &swept, index, sense)?;
            plan.face(PlanFace {
                surface,
                sense,
                name,
                origin: Some(origin),
                loops,
            });
            continue;
        }
        let (low_edge, high_edge) = if frame.full {
            let edge = plan.edge(
                lifted.transformed(&frame.low)?,
                piece.range(),
                ends(&low_vertices)?,
                EdgeName::seam(name),
            );
            (edge, edge)
        } else {
            let low_edge = plan.edge(
                lifted.transformed(&frame.low)?,
                piece.range(),
                ends(&low_vertices)?,
                EdgeName::between(name, caps.low.0),
            );
            let high_edge = plan.edge(
                lifted.transformed(&frame.high)?,
                piece.range(),
                ends(&high_vertices)?,
                EdgeName::between(name, caps.high.0),
            );
            low_loop.push(PlanCoedge::new(low_edge, travel.reversed()));
            high_loop.push(PlanCoedge::new(high_edge, travel));
            (low_edge, high_edge)
        };
        let mapped = entry.kind == Kind::Curved { mapped: true };
        let range = piece.range();
        let sweep = frame.sweep;
        let coedge = |edge: usize, sense: Sense, from: Point2, to: Point2| {
            if mapped {
                PlanCoedge::mapped(edge, sense, from, to)
            } else {
                PlanCoedge::new(edge, sense)
            }
        };
        let mut coedges = vec![coedge(
            low_edge,
            travel,
            Point2::new(0.0, range.start()),
            Point2::new(0.0, range.end()),
        )];
        if let Some(edge) = swept_at(index + 1) {
            let height = piece.end_parameter();
            coedges.push(coedge(
                edge,
                Sense::Same,
                Point2::new(0.0, height),
                Point2::new(sweep, height),
            ));
        }
        coedges.push(coedge(
            high_edge,
            travel.reversed(),
            Point2::new(sweep, range.start()),
            Point2::new(sweep, range.end()),
        ));
        if let Some(edge) = swept_at(index) {
            let height = piece.start_parameter();
            coedges.insert(
                0,
                coedge(
                    edge,
                    Sense::Reversed,
                    Point2::new(0.0, height),
                    Point2::new(sweep, height),
                ),
            );
        }
        plan.face(PlanFace {
            surface,
            sense,
            name,
            origin: Some(origin),
            loops: vec![coedges],
        });
    }
    low_loop.reverse();
    Ok((low_loop, high_loop))
}

fn flat_disc_loops(
    frame: &Frame,
    joints: &[Point2],
    swept: &[Option<usize>],
    index: usize,
    sense: Sense,
) -> Result<Vec<Vec<PlanCoedge>>, SweepError> {
    let count = joints.len().max(1);
    let next = (index + 1) % count;
    let radius = |joint: usize| {
        joints
            .get(joint)
            .map_or(0.0, |point| frame.axis.signed_distance(*point))
    };
    let (outer, inner) = if radius(index) >= radius(next) {
        (index, next)
    } else {
        (next, index)
    };
    let circle = |joint: usize| swept.get(joint).copied().flatten();
    let outer_edge = circle(outer).ok_or(SweepError::Unassembled)?;
    let mut loops = vec![vec![PlanCoedge::new(outer_edge, sense)]];
    if let Some(inner_edge) = circle(inner) {
        loops.push(vec![PlanCoedge::new(inner_edge, sense.reversed())]);
    }
    Ok(loops)
}
