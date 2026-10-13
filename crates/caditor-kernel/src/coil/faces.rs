use std::f64::consts::TAU;

use caditor_geometry::{Plane, Point2, Point3, Vector3};

use super::{COIL_PARTS, CoilError, Layout, SECTION_PIECES, SectionPiece};
use crate::{
    bspline::BSpline,
    build::plan::{Plan, PlanCoedge, PlanError, PlanFace, outward_sense},
    curve::Curve,
    error::GeometryError,
    interrupt,
    interval::Interval,
    naming::{EdgeName, FaceName, FaceOrigin},
    sense::Sense,
    surface::{BSplineSurface, Cylinder, PlaneSurface, Surface},
    thread::{SPANS_PER_TURN, helix_knots},
    topology::Solid,
};

const PROBE_FRACTION: f64 = 0.5;
const TANGENT_STEP: f64 = 1e-3;
const ROUNDING_SLACK: f64 = 1e-9;
const LINE_KNOTS: [f64; 4] = [0.0, 0.0, 1.0, 1.0];
const ARC_KNOTS: [f64; 6] = [0.0, 0.0, 0.0, 1.0, 1.0, 1.0];

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

struct Motion<'a> {
    layout: &'a Layout,
}

impl Motion<'_> {
    fn radial(&self, angle: f64) -> Vector3 {
        let frame = &self.layout.frame;
        frame.x_axis() * angle.cos() + frame.y_axis() * (self.layout.hand.sign() * angle.sin())
    }

    fn tangential(&self, angle: f64) -> Vector3 {
        let frame = &self.layout.frame;
        frame.x_axis() * -angle.sin() + frame.y_axis() * (self.layout.hand.sign() * angle.cos())
    }

    fn rise(&self, angle: f64) -> f64 {
        self.layout.pitch * angle / TAU
    }

    fn control(&self, angle: f64, at: Point2, stretch: f64) -> Point3 {
        let frame = &self.layout.frame;
        frame.origin()
            + self.radial(angle) * (stretch * (at.x + self.layout.slope * angle))
            + frame.normal() * (self.rise(angle) + at.y)
    }

    fn point(&self, angle: f64, at: Point2) -> Point3 {
        self.control(angle, at, 1.0)
    }

    fn net(&self, at: Point2, weight: f64, piece: Piece) -> (Vec<Point3>, Vec<f64>) {
        let half = 0.5 * piece.step;
        let middle_weight = half.cos();
        let count = 2 * piece.spans + 1;
        let mut points = Vec::with_capacity(count);
        let mut weights = Vec::with_capacity(count);
        for span in 0..piece.spans {
            let start = piece.angle(span as f64);
            points.push(self.point(start, at));
            weights.push(weight);
            points.push(self.control(start + half, at, 1.0 / middle_weight));
            weights.push(weight * middle_weight);
        }
        points.push(self.point(piece.end(), at));
        weights.push(weight);
        (points, weights)
    }

    fn helix(&self, at: Point2, piece: Piece) -> Result<Curve, CoilError> {
        let (points, weights) = self.net(at, 1.0, piece);
        Ok(Curve::BSpline(BSpline::rational(
            2,
            helix_knots(piece.spans),
            points,
            weights,
        )?))
    }

    fn joint(&self, angle: f64, section: &SectionPiece) -> Result<Curve, CoilError> {
        let points = section
            .points
            .iter()
            .map(|at| self.point(angle, *at))
            .collect();
        Ok(Curve::BSpline(BSpline::rational(
            2,
            ARC_KNOTS.to_vec(),
            points,
            section.weights.clone(),
        )?))
    }

    fn is_round(&self, section: &SectionPiece) -> bool {
        section.is_upright() && self.layout.slope == 0.0
    }

    fn side_surface(&self, section: &SectionPiece, piece: Piece) -> Result<Surface, CoilError> {
        if self.is_round(section) {
            let frame = &self.layout.frame;
            let axis = Plane::from_frame(frame.origin(), frame.normal(), frame.x_axis())
                .ok_or(GeometryError::DegenerateSurface)?;
            return Ok(Surface::Cylinder(Cylinder::new(axis, section.start().x)?));
        }
        let mut points = Vec::new();
        let mut weights = Vec::new();
        let mut columns = 0;
        for (at, weight) in section.points.iter().zip(&section.weights) {
            let (row, row_weights) = self.net(*at, *weight, piece);
            columns = row.len();
            points.extend(row);
            weights.extend(row_weights);
        }
        let (v_degree, v_knots) = if section.is_arc() {
            (2, ARC_KNOTS.to_vec())
        } else {
            (1, LINE_KNOTS.to_vec())
        };
        Ok(Surface::from(BSplineSurface::new(
            2,
            v_degree,
            helix_knots(piece.spans),
            v_knots,
            columns,
            points,
            Some(weights),
        )?))
    }

    fn meridian(&self, angle: f64) -> Result<Surface, CoilError> {
        let frame = &self.layout.frame;
        let plane = Plane::from_frame(
            frame.origin() + frame.normal() * self.rise(angle),
            self.tangential(angle),
            self.radial(angle),
        )
        .ok_or(GeometryError::DegenerateSurface)?;
        Ok(Surface::Plane(PlaneSurface::new(plane)?))
    }
}

fn pieces(turns: f64) -> Vec<Piece> {
    let count = (turns - ROUNDING_SLACK).ceil().max(1.0);
    let length = turns / count;
    let spans = ((length * SPANS_PER_TURN as f64 - ROUNDING_SLACK).ceil() as usize).max(1);
    (0..count as usize)
        .map(|index| Piece {
            start: TAU * length * index as f64,
            spans,
            step: TAU * length / spans as f64,
        })
        .collect()
}

fn next(index: usize) -> usize {
    (index + 1) % SECTION_PIECES
}

fn pick<T: Copy>(items: &[T], index: usize) -> Result<T, CoilError> {
    items.get(index).copied().ok_or(CoilError::Unassembled)
}

fn ring_of<T: Copy>(
    rings: &[[T; SECTION_PIECES]],
    index: usize,
) -> Result<[T; SECTION_PIECES], CoilError> {
    rings.get(index).copied().ok_or(CoilError::Unassembled)
}

fn section_of(layout: &Layout, index: usize) -> Result<&SectionPiece, CoilError> {
    layout.section.get(index).ok_or(CoilError::Unassembled)
}

pub(super) fn build(layout: &Layout, feature: u64) -> Result<Solid, CoilError> {
    let motion = Motion { layout };
    let pieces = pieces(layout.turns);
    let parts = COIL_PARTS;
    let name = |part: u64| FaceName::coil(feature, part);
    let start_name = name(parts.start);
    let end_name = name(parts.end);
    let side_name = |turn: usize, index: usize| name(parts.side(turn, index));

    let mut boundaries: Vec<f64> = pieces.iter().map(|piece| piece.start).collect();
    boundaries.push(layout.sweep());
    let mut plan = Plan::default();
    let corners: [Point2; SECTION_PIECES] = layout.section.each_ref().map(SectionPiece::start);
    let vertices: Vec<[usize; SECTION_PIECES]> = boundaries
        .iter()
        .map(|angle| corners.map(|corner| plan.vertex(motion.point(*angle, corner))))
        .collect();

    let mut joints: Vec<[usize; SECTION_PIECES]> = Vec::with_capacity(boundaries.len());
    for (index, (ring, angle)) in vertices.iter().zip(&boundaries).enumerate() {
        let mut ring_joints = [0; SECTION_PIECES];
        for (side, slot) in ring_joints.iter_mut().enumerate() {
            let edge_name = if index == 0 {
                EdgeName::between(side_name(0, side), start_name)
            } else if index == pieces.len() {
                EdgeName::between(side_name(index - 1, side), end_name)
            } else {
                EdgeName::between(side_name(index - 1, side), side_name(index, side))
            };
            let ends = (pick(ring, side)?, pick(ring, next(side))?);
            let section = section_of(layout, side)?;
            *slot = if section.is_arc() {
                let interval = Interval::new(0.0, 1.0).ok_or(CoilError::Unassembled)?;
                plan.edge(motion.joint(*angle, section)?, interval, ends, edge_name)
            } else {
                plan.line(ends.0, ends.1, edge_name)
                    .map_err(|_| CoilError::Unassembled)?
            };
        }
        joints.push(ring_joints);
    }

    let mut helices: Vec<[usize; SECTION_PIECES]> = Vec::with_capacity(pieces.len());
    for (turn, piece) in pieces.iter().enumerate() {
        interrupt::check()?;
        let (from, to) = (ring_of(&vertices, turn)?, ring_of(&vertices, turn + 1)?);
        let interval = Interval::new(0.0, piece.spans as f64).ok_or(CoilError::Unassembled)?;
        let mut ring_helices = [0; SECTION_PIECES];
        for (corner, slot) in ring_helices.iter_mut().enumerate() {
            let before = side_name(turn, (corner + SECTION_PIECES - 1) % SECTION_PIECES);
            let after = side_name(turn, corner);
            *slot = plan.edge(
                motion.helix(pick(&corners, corner)?, *piece)?,
                interval,
                (pick(&from, corner)?, pick(&to, corner)?),
                EdgeName::between(before, after),
            );
        }
        helices.push(ring_helices);
    }

    for (turn, piece) in pieces.iter().enumerate() {
        interrupt::check()?;
        let ring = ring_of(&helices, turn)?;
        let (starts, ends) = (ring_of(&joints, turn)?, ring_of(&joints, turn + 1)?);
        for index in 0..SECTION_PIECES {
            let side = Side {
                section: section_of(layout, index)?,
                low: pick(&ring, index)?,
                high: pick(&ring, next(index))?,
                start: pick(&starts, index)?,
                end: pick(&ends, index)?,
            };
            let part = parts.side(turn, index);
            plan.face(side_face(&motion, *piece, &side, part, feature)?);
        }
    }

    let last = pieces.len();
    for (ring, angle, part, outward_along) in [
        (ring_of(&joints, 0)?, 0.0, parts.start, -1.0),
        (ring_of(&joints, last)?, layout.sweep(), parts.end, 1.0),
    ] {
        plan.face(cap_face(
            &motion,
            &corners,
            angle,
            ring,
            outward_along,
            part,
            feature,
        )?);
    }
    plan.build().map_err(|error| match error {
        PlanError::Build(error) | PlanError::Labelled { error, .. } => CoilError::from(error),
        PlanError::Unassembled => CoilError::Unassembled,
    })
}

struct Side<'a> {
    section: &'a SectionPiece,
    low: usize,
    high: usize,
    start: usize,
    end: usize,
}

fn side_face(
    motion: &Motion<'_>,
    piece: Piece,
    side: &Side<'_>,
    part: u64,
    feature: u64,
) -> Result<PlanFace, CoilError> {
    let surface = motion.side_surface(side.section, piece)?;
    let round = motion.is_round(side.section);
    let local = PROBE_FRACTION * piece.spans as f64;
    let probe_angle = piece.angle(local);
    let on_face = motion.point(probe_angle, side.section.middle());
    let inside = motion.point(probe_angle, motion.layout.centre);
    let hint = (!round).then(|| Point2::new(local, 0.5));
    let sense = outward_sense(&surface, on_face, on_face - inside, hint);
    let uv = surface.project(on_face, hint);
    let normal = surface.normal(uv.x, uv.y).unwrap_or(Vector3::ZERO) * sense.sign();
    let from = side.section.start();
    let along = motion.point(probe_angle + TANGENT_STEP * piece.step, from)
        - motion.point(probe_angle, from);
    let across = motion.point(probe_angle, side.section.end()) - motion.point(probe_angle, from);
    let forward = normal.cross(along).dot(across) > 0.0;

    let top = piece.spans as f64;
    let coedge = |edge: usize, at_start: Point2, at_end: Point2| {
        if round {
            PlanCoedge::new(edge, Sense::Same)
        } else {
            PlanCoedge::mapped(edge, Sense::Same, at_start, at_end)
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
        name: FaceName::coil(feature, part),
        origin: Some(FaceOrigin::Side {
            feature,
            entity: part,
        }),
        loops: vec![face_loop],
    })
}

fn cap_face(
    motion: &Motion<'_>,
    corners: &[Point2; SECTION_PIECES],
    angle: f64,
    joints: [usize; SECTION_PIECES],
    outward_along: f64,
    part: u64,
    feature: u64,
) -> Result<PlanFace, CoilError> {
    let surface = motion.meridian(angle)?;
    let points = corners.map(|corner| motion.point(angle, corner));
    let centre = motion.point(angle, motion.layout.centre);
    let sense = outward_sense(
        &surface,
        centre,
        motion.tangential(angle) * outward_along,
        None,
    );
    let uv = surface.project(centre, None);
    let normal = surface.normal(uv.x, uv.y).unwrap_or(Vector3::ZERO) * sense.sign();
    let turning = (0..SECTION_PIECES)
        .map(|index| -> Result<Vector3, CoilError> {
            Ok((pick(&points, index)? - centre).cross(pick(&points, next(index))? - centre))
        })
        .sum::<Result<Vector3, CoilError>>()?;
    let coedges = if turning.dot(normal) > 0.0 {
        joints
            .iter()
            .map(|joint| PlanCoedge::new(*joint, Sense::Same))
            .collect()
    } else {
        joints
            .iter()
            .rev()
            .map(|joint| PlanCoedge::new(*joint, Sense::Reversed))
            .collect()
    };
    Ok(PlanFace {
        surface,
        sense,
        name: FaceName::coil(feature, part),
        origin: Some(FaceOrigin::Side {
            feature,
            entity: part,
        }),
        loops: vec![coedges],
    })
}
