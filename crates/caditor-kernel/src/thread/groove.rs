use std::f64::consts::TAU;

use caditor_geometry::{Plane, Point2, Point3, Vector3};

use super::{Groove, ThreadError};
use crate::{
    bspline::BSpline,
    build::plan::{Plan, PlanCoedge, PlanError, PlanFace, outward_sense},
    curve::Curve,
    error::GeometryError,
    interval::Interval,
    naming::{EdgeName, FaceName, FaceOrigin},
    sense::Sense,
    surface::{BSplineSurface, Cylinder, PlaneSurface, Surface},
    topology::Solid,
};

pub const SPANS_PER_TURN: usize = 16;
pub const GROOVE_PARTS: GrooveParts = GrooveParts {
    root: 1,
    leading_flank: 2,
    far: 3,
    trailing_flank: 4,
    start: 5,
    end: 6,
    per_turn: 8,
};
const CORNERS: usize = 4;
const PROBE_FRACTION: f64 = 0.5;
const TANGENT_STEP: f64 = 1e-3;
const LEAST_PIECE: f64 = 0.05;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GrooveParts {
    pub root: u64,
    pub leading_flank: u64,
    pub far: u64,
    pub trailing_flank: u64,
    pub start: u64,
    pub end: u64,
    pub per_turn: u64,
}

impl GrooveParts {
    pub fn side(self, turn: usize, side: u64) -> u64 {
        side.wrapping_add(self.per_turn.wrapping_mul(turn as u64))
    }

    pub fn turn_of(self, part: u64) -> (u64, u64) {
        (part / self.per_turn, part % self.per_turn)
    }

    fn sides(self) -> [u64; CORNERS] {
        [self.root, self.leading_flank, self.far, self.trailing_flank]
    }
}

#[derive(Debug, Clone, Copy)]
struct Corner {
    radius: f64,
    offset: f64,
}

#[derive(Debug, Clone, Copy)]
struct Piece {
    start: f64,
    spans: usize,
    step: f64,
}

impl Piece {
    fn angle(&self, span: f64) -> f64 {
        self.start + self.step * span
    }

    fn end(&self) -> f64 {
        self.angle(self.spans as f64)
    }
}

struct Sweep<'a> {
    groove: &'a Groove,
}

impl Sweep<'_> {
    fn radial(&self, angle: f64) -> Vector3 {
        let frame = &self.groove.frame;
        frame.x_axis() * angle.cos() + frame.y_axis() * (self.groove.hand.sign() * angle.sin())
    }

    fn tangential(&self, angle: f64) -> Vector3 {
        let frame = &self.groove.frame;
        frame.x_axis() * -angle.sin() + frame.y_axis() * (self.groove.hand.sign() * angle.cos())
    }

    fn centre_at(&self, angle: f64) -> f64 {
        self.groove.first_centre + self.groove.pitch * angle / TAU
    }

    fn at(&self, corner: Corner, angle: f64, reach: f64) -> Point3 {
        let frame = &self.groove.frame;
        frame.origin()
            + self.radial(angle) * reach
            + frame.normal() * (self.centre_at(angle) + corner.offset)
    }

    fn point(&self, corner: Corner, angle: f64) -> Point3 {
        self.at(corner, angle, corner.radius)
    }

    fn control_net(&self, corner: Corner, piece: Piece) -> (Vec<Point3>, Vec<f64>) {
        let half = 0.5 * piece.step;
        let middle_weight = half.cos();
        let count = 2 * piece.spans + 1;
        let mut points = Vec::with_capacity(count);
        let mut weights = Vec::with_capacity(count);
        for span in 0..piece.spans {
            let start = piece.angle(span as f64);
            points.push(self.point(corner, start));
            weights.push(1.0);
            points.push(self.at(corner, start + half, corner.radius / middle_weight));
            weights.push(middle_weight);
        }
        points.push(self.point(corner, piece.end()));
        weights.push(1.0);
        (points, weights)
    }

    fn helix(&self, corner: Corner, piece: Piece) -> Result<Curve, ThreadError> {
        let (points, weights) = self.control_net(corner, piece);
        Ok(Curve::BSpline(BSpline::rational(
            2,
            knots(piece.spans),
            points,
            weights,
        )?))
    }

    fn flank(&self, from: Corner, to: Corner, piece: Piece) -> Result<Surface, ThreadError> {
        let (mut points, mut weights) = self.control_net(from, piece);
        let (far_points, far_weights) = self.control_net(to, piece);
        let columns = points.len();
        points.extend(far_points);
        weights.extend(far_weights);
        Ok(Surface::from(BSplineSurface::new(
            2,
            1,
            knots(piece.spans),
            vec![0.0, 0.0, 1.0, 1.0],
            columns,
            points,
            Some(weights),
        )?))
    }

    fn cylinder(&self, radius: f64) -> Result<Surface, ThreadError> {
        let frame = &self.groove.frame;
        let axis = Plane::from_frame(frame.origin(), frame.normal(), frame.x_axis())
            .ok_or(GeometryError::DegenerateSurface)?;
        Ok(Surface::Cylinder(Cylinder::new(axis, radius)?))
    }

    fn meridian(&self, angle: f64) -> Result<Surface, ThreadError> {
        let frame = &self.groove.frame;
        let plane = Plane::from_frame(
            frame.origin() + frame.normal() * self.centre_at(angle),
            self.tangential(angle),
            self.radial(angle),
        )
        .ok_or(GeometryError::DegenerateSurface)?;
        Ok(Surface::Plane(PlaneSurface::new(plane)?))
    }
}

fn knots(spans: usize) -> Vec<f64> {
    let mut knots = vec![0.0; 3];
    for span in 1..spans {
        knots.extend([span as f64; 2]);
    }
    knots.extend([spans as f64; 3]);
    knots
}

fn corners(groove: &Groove) -> [Corner; CORNERS] {
    [
        Corner {
            radius: groove.root_radius,
            offset: -groove.root_half_width,
        },
        Corner {
            radius: groove.root_radius,
            offset: groove.root_half_width,
        },
        Corner {
            radius: groove.far_radius,
            offset: groove.far_half_width,
        },
        Corner {
            radius: groove.far_radius,
            offset: -groove.far_half_width,
        },
    ]
}

fn pieces(groove: &Groove) -> Vec<Piece> {
    let mut stops: Vec<f64> = groove
        .crossings
        .iter()
        .copied()
        .filter(|turn| *turn > LEAST_PIECE && *turn < groove.turns - LEAST_PIECE)
        .collect();
    stops.push(0.0);
    stops.push(groove.turns);
    stops.sort_by(f64::total_cmp);
    stops.dedup();
    let mut pieces = Vec::new();
    for pair in stops.windows(2) {
        let [from, to] = pair else {
            continue;
        };
        let count = (to - from).ceil().max(1.0);
        let length = (to - from) / count;
        let spans = ((length * SPANS_PER_TURN as f64).ceil() as usize).max(1);
        for index in 0..count as usize {
            pieces.push(Piece {
                start: TAU * (from + length * index as f64),
                spans,
                step: TAU * length / spans as f64,
            });
        }
    }
    pieces
}

fn next(index: usize) -> usize {
    (index + 1) % CORNERS
}

fn pick<T: Copy>(items: &[T], index: usize) -> Result<T, ThreadError> {
    items
        .get(index)
        .copied()
        .ok_or(ThreadError::GrooveUnassembled)
}

fn ring_of<T: Copy>(rings: &[[T; CORNERS]], index: usize) -> Result<[T; CORNERS], ThreadError> {
    pick(rings, index)
}

pub(super) fn tool(groove: &Groove, feature: u64) -> Result<Solid, ThreadError> {
    let sweep = Sweep { groove };
    let corners = corners(groove);
    let pieces = pieces(groove);
    let parts = GROOVE_PARTS;
    let name = |part: u64| FaceName::thread(feature, part);
    let start_name = name(parts.start);
    let end_name = name(parts.end);
    let side_name = |turn: usize, corner: usize| -> Result<FaceName, ThreadError> {
        Ok(name(parts.side(turn, pick(&parts.sides(), corner)?)))
    };

    let mut boundaries: Vec<f64> = pieces.iter().map(|piece| piece.start).collect();
    boundaries.push(groove.sweep());
    let mut plan = Plan::default();
    let vertices: Vec<[usize; CORNERS]> = boundaries
        .iter()
        .map(|angle| corners.map(|corner| plan.vertex(sweep.point(corner, *angle))))
        .collect();

    let mut lines: Vec<[usize; CORNERS]> = Vec::with_capacity(boundaries.len());
    for (index, ring) in vertices.iter().enumerate() {
        let mut ring_lines = [0; CORNERS];
        for (corner, slot) in ring_lines.iter_mut().enumerate() {
            let side = |turn: usize| side_name(turn, corner);
            let edge_name = if index == 0 {
                EdgeName::between(side(0)?, start_name)
            } else if index == pieces.len() {
                EdgeName::between(side(index - 1)?, end_name)
            } else {
                EdgeName::between(side(index - 1)?, side(index)?)
            };
            *slot = plan.line(pick(ring, corner)?, pick(ring, next(corner))?, edge_name)?;
        }
        lines.push(ring_lines);
    }

    let mut helices: Vec<[usize; CORNERS]> = Vec::with_capacity(pieces.len());
    for (turn, piece) in pieces.iter().enumerate() {
        let (from, to) = (ring_of(&vertices, turn)?, ring_of(&vertices, turn + 1)?);
        let interval = Interval::new(0.0, piece.spans as f64).ok_or(ThreadError::NoPitch)?;
        let mut ring_helices = [0; CORNERS];
        for (corner, slot) in ring_helices.iter_mut().enumerate() {
            let before = side_name(turn, (corner + CORNERS - 1) % CORNERS)?;
            let after = side_name(turn, corner)?;
            *slot = plan.edge(
                sweep.helix(pick(&corners, corner)?, *piece)?,
                interval,
                (pick(&from, corner)?, pick(&to, corner)?),
                EdgeName::between(before, after),
            );
        }
        helices.push(ring_helices);
    }

    for (turn, piece) in pieces.iter().enumerate() {
        let ring = ring_of(&helices, turn)?;
        let (starts, ends) = (ring_of(&lines, turn)?, ring_of(&lines, turn + 1)?);
        for corner in 0..CORNERS {
            let side = Side {
                from: pick(&corners, corner)?,
                to: pick(&corners, next(corner))?,
                flank: corner % 2 == 1,
                low: pick(&ring, corner)?,
                high: pick(&ring, next(corner))?,
                start: pick(&starts, corner)?,
                end: pick(&ends, corner)?,
            };
            let part = parts.side(turn, pick(&parts.sides(), corner)?);
            plan.face(side_face(&sweep, *piece, &side, name(part), part, feature)?);
        }
    }

    let last = pieces.len();
    for (ring, angle, part, outward_along) in [
        (ring_of(&lines, 0)?, 0.0, parts.start, -1.0),
        (ring_of(&lines, last)?, groove.sweep(), parts.end, 1.0),
    ] {
        plan.face(cap_face(
            &sweep,
            &corners,
            angle,
            ring,
            outward_along,
            name(part),
            FaceOrigin::Side {
                feature,
                entity: part,
            },
        )?);
    }
    plan.build().map_err(|error| match error {
        PlanError::Build(error) | PlanError::Labelled { error, .. } => ThreadError::from(error),
        PlanError::Unassembled => ThreadError::GrooveUnassembled,
    })
}

struct Side {
    from: Corner,
    to: Corner,
    flank: bool,
    low: usize,
    high: usize,
    start: usize,
    end: usize,
}

fn side_face(
    sweep: &Sweep<'_>,
    piece: Piece,
    side: &Side,
    name: FaceName,
    part: u64,
    feature: u64,
) -> Result<PlanFace, ThreadError> {
    let groove = sweep.groove;
    let surface = if side.flank {
        sweep.flank(side.from, side.to, piece)?
    } else {
        sweep.cylinder(side.from.radius)?
    };
    let local = PROBE_FRACTION * piece.spans as f64;
    let probe_angle = piece.angle(local);
    let middle = Corner {
        radius: 0.5 * (side.from.radius + side.to.radius),
        offset: 0.5 * (side.from.offset + side.to.offset),
    };
    let on_face = sweep.point(middle, probe_angle);
    let inside = sweep.point(
        Corner {
            radius: 0.5 * (groove.root_radius + groove.far_radius),
            offset: 0.0,
        },
        probe_angle,
    );
    let hint = side.flank.then(|| Point2::new(local, 0.5));
    let sense = outward_sense(&surface, on_face, on_face - inside, hint);
    let uv = surface.project(on_face, hint);
    let normal = surface.normal(uv.x, uv.y).unwrap_or(Vector3::ZERO) * sense.sign();
    let along = sweep.point(side.from, probe_angle + TANGENT_STEP * piece.step)
        - sweep.point(side.from, probe_angle);
    let across = sweep.point(side.to, probe_angle) - sweep.point(side.from, probe_angle);
    let forward = normal.cross(along).dot(across) > 0.0;

    let top = piece.spans as f64;
    let coedge = |edge: usize, at_start: Point2, at_end: Point2| {
        if side.flank {
            PlanCoedge::mapped(edge, Sense::Same, at_start, at_end)
        } else {
            PlanCoedge::new(edge, Sense::Same)
        }
    };
    let low = coedge(side.low, Point2::new(0.0, 0.0), Point2::new(top, 0.0));
    let high = coedge(side.high, Point2::new(0.0, 1.0), Point2::new(top, 1.0));
    let start = coedge(side.start, Point2::new(0.0, 0.0), Point2::new(0.0, 1.0));
    let end = coedge(side.end, Point2::new(top, 0.0), Point2::new(top, 1.0));
    let reversed = |coedge: PlanCoedge| PlanCoedge {
        sense: coedge.sense.reversed(),
        ..coedge
    };
    let face_loop = if forward {
        vec![low, end, reversed(high), reversed(start)]
    } else {
        vec![start, high, reversed(end), reversed(low)]
    };
    Ok(PlanFace {
        surface,
        sense,
        name,
        origin: Some(FaceOrigin::Side {
            feature,
            entity: part,
        }),
        loops: vec![face_loop],
    })
}

fn cap_face(
    sweep: &Sweep<'_>,
    corners: &[Corner; CORNERS],
    angle: f64,
    lines: [usize; CORNERS],
    outward_along: f64,
    name: FaceName,
    origin: FaceOrigin,
) -> Result<PlanFace, ThreadError> {
    let surface = sweep.meridian(angle)?;
    let points = corners.map(|corner| sweep.point(corner, angle));
    let sum = points
        .iter()
        .fold(Vector3::ZERO, |sum, point| sum + (*point - Point3::ZERO));
    let centre = Point3::ZERO + sum / CORNERS as f64;
    let sense = outward_sense(
        &surface,
        centre,
        sweep.tangential(angle) * outward_along,
        None,
    );
    let uv = surface.project(centre, None);
    let normal = surface.normal(uv.x, uv.y).unwrap_or(Vector3::ZERO) * sense.sign();
    let turning = (0..CORNERS)
        .map(|index| -> Result<Vector3, ThreadError> {
            Ok((pick(&points, index)? - centre).cross(pick(&points, next(index))? - centre))
        })
        .sum::<Result<Vector3, ThreadError>>()?;
    let coedges = if turning.dot(normal) > 0.0 {
        lines
            .iter()
            .map(|line| PlanCoedge::new(*line, Sense::Same))
            .collect()
    } else {
        lines
            .iter()
            .rev()
            .map(|line| PlanCoedge::new(*line, Sense::Reversed))
            .collect()
    };
    Ok(PlanFace {
        surface,
        sense,
        name,
        origin: Some(origin),
        loops: vec![coedges],
    })
}
