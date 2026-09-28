use caditor_geometry::{Plane, Point2};

use crate::{
    build::{
        LinearExtent, SweepError, lift,
        plan::{Plan, PlanCoedge, PlanFace, outward_sense},
        traversal_sense, traversal_tangent,
    },
    curve2::Curve2,
    error::GeometryError,
    naming::{EdgeName, FaceName, FaceOrigin},
    profile::{Piece, Region},
    sense::Sense,
    surface::{Cylinder, Extrusion, PlaneSurface, Surface},
    topology::Solid,
};

fn offset_plane(plane: &Plane, offset: f64) -> Result<Plane, SweepError> {
    Plane::from_frame(
        plane.origin() + plane.normal() * offset,
        plane.normal(),
        plane.x_axis(),
    )
    .ok_or(SweepError::Geometry(GeometryError::ZeroDirection))
}

struct Caps {
    low: (FaceName, FaceOrigin),
    high: (FaceName, FaceOrigin),
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
    let low = extent.start().min(extent.end());
    let high = extent.start().max(extent.end());
    let height = high - low;
    let bottom = offset_plane(plane, low)?;
    let top = offset_plane(plane, high)?;
    let mut plan = Plan::default();
    for region in regions {
        let start = (
            FaceName::start_cap(feature, region.key()),
            FaceOrigin::StartCap { feature },
        );
        let end = (
            FaceName::end_cap(feature, region.key()),
            FaceOrigin::EndCap { feature },
        );
        let caps = if extent.start() <= extent.end() {
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
            let (bottom_loop, top_loop) = extrude_loop(
                &mut plan,
                &bottom,
                &top,
                height,
                profile_loop.pieces(),
                &caps,
                feature,
            )?;
            bottom_loops.push(bottom_loop);
            top_loops.push(top_loop);
        }
        plan.face(PlanFace {
            surface: PlaneSurface::new(bottom.flipped())?.into(),
            sense: Sense::Same,
            name: caps.low.0,
            origin: Some(caps.low.1),
            loops: bottom_loops,
        });
        plan.face(PlanFace {
            surface: PlaneSurface::new(top)?.into(),
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
    bottom: &Plane,
    top: &Plane,
    height: f64,
    pieces: &[Piece],
    caps: &Caps,
    feature: u64,
) -> Result<(Vec<PlanCoedge>, Vec<PlanCoedge>), SweepError> {
    let count = pieces.len();
    let joints: Vec<Point2> = pieces.iter().map(Piece::start).collect();
    let lower: Vec<usize> = joints
        .iter()
        .map(|joint| plan.vertex(bottom.to_world(*joint)))
        .collect();
    let upper: Vec<usize> = joints
        .iter()
        .map(|joint| plan.vertex(top.to_world(*joint)))
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
        let bottom_edge = plan.edge(
            piece.curve().on_plane(bottom)?,
            piece.range(),
            ends(&lower)?,
            EdgeName::between(side, caps.low.0),
        );
        let top_edge = plan.edge(
            piece.curve().on_plane(top)?,
            piece.range(),
            ends(&upper)?,
            EdgeName::between(side, caps.high.0),
        );
        let (surface, mapped) = side_surface(bottom, piece)?;
        let middle = piece.range().middle();
        let probe = bottom.to_world(piece.curve().point(middle)) + bottom.normal() * (0.5 * height);
        let outward = lift(bottom, traversal_tangent(piece, middle)).cross(bottom.normal());
        let near = mapped.then(|| Point2::new(middle, 0.5 * height));
        let sense = outward_sense(&surface, probe, outward, near);
        let (entering, leaving) = (at(&verticals, index)?, at(&verticals, index + 1)?);
        let range = piece.range();
        let loop_coedges = if mapped {
            let (u_enter, u_leave) = (piece.start_parameter(), piece.end_parameter());
            vec![
                PlanCoedge::mapped(
                    bottom_edge,
                    travel,
                    Point2::new(range.start(), 0.0),
                    Point2::new(range.end(), 0.0),
                ),
                PlanCoedge::mapped(
                    leaving,
                    Sense::Same,
                    Point2::new(u_leave, 0.0),
                    Point2::new(u_leave, height),
                ),
                PlanCoedge::mapped(
                    top_edge,
                    travel.reversed(),
                    Point2::new(range.start(), height),
                    Point2::new(range.end(), height),
                ),
                PlanCoedge::mapped(
                    entering,
                    Sense::Reversed,
                    Point2::new(u_enter, 0.0),
                    Point2::new(u_enter, height),
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

fn side_surface(bottom: &Plane, piece: &Piece) -> Result<(Surface, bool), SweepError> {
    let normal = bottom.normal();
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
