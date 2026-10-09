use std::collections::BTreeSet;

use caditor_geometry::{Aabb2, Plane, Point2, Vector3};
use thiserror::Error;

use crate::{
    boolean::{BooleanError, BooleanOperation, boolean},
    build::{
        LinearBound, SweepError,
        extrude::{Level, span},
    },
    interrupt::{self, Interrupted},
    profile::Region,
    surface::Surface,
    tessellation::TessellationError,
    tolerance::{LINEAR_RESOLUTION, SamplingTolerance, same_direction},
    topology::{FaceId, PointClass, RayCrossing, ShellId, Solid},
};

const RAY_SAMPLES: usize = 256;
const MAX_RAYS: usize = 1024;
const MAX_SUBDIVISIONS: usize = 16;
const MAX_VERTEX_RAYS: usize = 1024;
const VERTEX_NUDGE: f64 = 2e-4;
const NUDGES: [(f64, f64); 4] = [(1.0, 1.0), (-1.0, 1.0), (-1.0, -1.0), (1.0, -1.0)];
const RAY_START: f64 = 4.0 * LINEAR_RESOLUTION;
const MAX_START_PROBES: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Heights {
    pub least: f64,
    pub most: f64,
}

pub fn heights(plane: &Plane, regions: &[Region], target: &Plane) -> Result<Heights, SweepError> {
    let level = Level::of(plane, LinearBound::Plane(*target))?;
    let (least, most) = span(&level, regions).ok_or(SweepError::NoRegions)?;
    Ok(Heights { least, most })
}

pub fn heights_along(
    plane: &Plane,
    regions: &[Region],
    target: &Plane,
    direction: Vector3,
) -> Result<Heights, SweepError> {
    let rise = direction.dot(plane.normal());
    if !rise.is_finite() || rise.abs() <= LINEAR_RESOLUTION * direction.length() {
        return Err(SweepError::DirectionAlongSketch);
    }
    let level = Level::along(plane, direction / rise, LinearBound::Plane(*target))?;
    let (least, most) = span(&level, regions).ok_or(SweepError::NoRegions)?;
    Ok(Heights { least, most })
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NextFace {
    pub face: FaceId,
    pub plane: Plane,
    pub entering: bool,
}

#[derive(Debug, Clone, PartialEq, Error)]
pub enum ReachError {
    #[error("no region is chosen")]
    NoRegions,
    #[error("the profile meets nothing of the solid in this direction")]
    Nothing,
    #[error("part of the profile passes beside the solid")]
    Partly,
    #[error("the profile meets several faces of the solid first")]
    SeveralFaces(Vec<FaceId>),
    #[error("the first face the profile meets is not flat")]
    Curved(FaceId),
    #[error("no ray from the profile crosses the solid cleanly")]
    Undecided,
    #[error(transparent)]
    Cancelled(#[from] Interrupted),
}

fn flat_plane(solid: &Solid, face: FaceId) -> Option<Plane> {
    let face = solid.face(face)?;
    let Surface::Plane(surface) = face.surface() else {
        return None;
    };
    let frame = *surface.frame();
    Some(if face.sense().is_same() {
        frame
    } else {
        frame.flipped()
    })
}

fn same_plane(first: &Plane, second: &Plane) -> bool {
    same_direction(first.normal(), second.normal())
        && first.signed_distance(second.origin()).abs() <= LINEAR_RESOLUTION
}

fn centroids([a, b, c]: [Point2; 3], divisions: usize) -> Vec<Point2> {
    let steps = divisions as f64;
    let at = |along: f64, across: f64| a + (b - a) * (along / steps) + (c - a) * (across / steps);
    let mut points = Vec::with_capacity(divisions * divisions);
    for along in 0..divisions {
        for across in 0..divisions - along {
            let (along_step, across_step) = (along as f64, across as f64);
            points.push(at(along_step + 1.0 / 3.0, across_step + 1.0 / 3.0));
            if along + across + 1 < divisions {
                points.push(at(along_step + 2.0 / 3.0, across_step + 2.0 / 3.0));
            }
        }
    }
    points
}

fn ray_origins(regions: &[Region]) -> Vec<Point2> {
    let size = profile_size(regions);
    let tolerance = SamplingTolerance::for_extent(size);
    let triangles: Vec<[Point2; 3]> = regions
        .iter()
        .filter_map(|region| region.triangulate(&tolerance))
        .flat_map(|mesh| {
            mesh.triangles
                .iter()
                .filter_map(|triangle| {
                    let [a, b, c] = triangle.map(|index| mesh.points.get(index as usize).copied());
                    Some([a?, b?, c?])
                })
                .collect::<Vec<_>>()
        })
        .collect();
    let area = |[a, b, c]: &[Point2; 3]| 0.5 * (b - a).perp_dot(c - a).abs();
    let total: f64 = triangles.iter().map(area).sum();
    if total.is_nan() || total <= 0.0 {
        return Vec::new();
    }
    let origins: Vec<Point2> = triangles
        .iter()
        .flat_map(|triangle| {
            let share = RAY_SAMPLES as f64 * area(triangle) / total;
            let divisions = (share.sqrt().ceil() as usize).clamp(1, MAX_SUBDIVISIONS);
            centroids(*triangle, divisions)
        })
        .collect();
    let stride = origins.len().div_ceil(MAX_RAYS).max(1);
    origins.into_iter().step_by(stride).collect()
}

fn profile_size(regions: &[Region]) -> f64 {
    regions
        .iter()
        .filter_map(Region::bounds)
        .reduce(Aabb2::union)
        .map_or(1.0, |bounds| bounds.size().length())
}

fn vertex_origins(solid: &Solid, plane: &Plane, regions: &[Region]) -> Vec<Point2> {
    let nudge = VERTEX_NUDGE * profile_size(regions);
    let origins: Vec<Point2> = solid
        .vertices()
        .map(|(_, vertex)| plane.to_local(vertex.point()))
        .flat_map(|projected| {
            NUDGES
                .iter()
                .map(move |(across, along)| projected + Point2::new(*across, *along) * nudge)
        })
        .filter(|origin| regions.iter().any(|region| region.contains(*origin)))
        .collect();
    let stride = origins.len().div_ceil(MAX_VERTEX_RAYS).max(1);
    origins.into_iter().step_by(stride).collect()
}

pub fn next_face(
    solid: &Solid,
    plane: &Plane,
    regions: &[Region],
    reversed: bool,
) -> Result<NextFace, ReachError> {
    let direction = if reversed {
        -plane.normal()
    } else {
        plane.normal()
    };
    let mut origins = ray_origins(regions);
    if origins.is_empty() {
        return Err(ReachError::NoRegions);
    }
    origins.extend(vertex_origins(solid, plane, regions));
    let classifier = solid.classifier();
    let mut met: Vec<(FaceId, bool)> = Vec::new();
    let (mut decided, mut missed) = (0_usize, 0_usize);
    for origin in origins {
        interrupt::check()?;
        match classifier.first_crossing(plane.to_world(origin), direction, RAY_START) {
            RayCrossing::Crossing { face, entering, .. } => {
                decided += 1;
                if met.iter().all(|(known, _)| *known != face) {
                    met.push((face, entering));
                }
            }
            RayCrossing::Nothing => {
                decided += 1;
                missed += 1;
            }
            RayCrossing::Undecided => {}
        }
    }
    interrupt::check()?;
    if decided == 0 {
        return Err(ReachError::Undecided);
    }
    if missed == decided {
        return Err(ReachError::Nothing);
    }
    if missed > 0 {
        return Err(ReachError::Partly);
    }
    let mut groups: Vec<(FaceId, Option<Plane>, bool)> = Vec::new();
    for (face, entering) in met {
        let plane = flat_plane(solid, face);
        let joins = groups.iter().any(|(_, known, _)| match (known, &plane) {
            (Some(known), Some(plane)) => same_plane(known, plane),
            _ => false,
        });
        if !joins {
            groups.push((face, plane, entering));
        }
    }
    match groups.as_slice() {
        [(face, Some(plane), entering)] => Ok(NextFace {
            face: *face,
            plane: *plane,
            entering: *entering,
        }),
        [(face, None, _)] => Err(ReachError::Curved(*face)),
        _ => Err(ReachError::SeveralFaces(
            groups.iter().map(|(face, _, _)| *face).collect(),
        )),
    }
}

#[derive(Debug, Clone, PartialEq, Error)]
pub enum StopError {
    #[error("no region is chosen")]
    NoRegions,
    #[error("the profile starts partly inside the body and partly outside it")]
    Straddles,
    #[error("the body lies wholly inside the sweep")]
    Enclosed,
    #[error("part of the profile passes the body without stopping")]
    PassesBeside,
    #[error("nothing of the sweep lies before the body")]
    Nothing,
    #[error(transparent)]
    Boolean(#[from] BooleanError),
    #[error(transparent)]
    Tessellation(#[from] TessellationError),
    #[error(transparent)]
    Cancelled(#[from] Interrupted),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Stopped {
    pub solid: Solid,
    pub entering: bool,
}

pub fn stop_at_body(
    tool: &Solid,
    body: &Solid,
    plane: &Plane,
    regions: &[Region],
    reversed: bool,
    far: f64,
) -> Result<Stopped, StopError> {
    let direction = if reversed {
        -plane.normal()
    } else {
        plane.normal()
    };
    let origins = ray_origins(regions);
    if origins.is_empty() {
        return Err(StopError::NoRegions);
    }
    let stride = origins.len().div_ceil(MAX_START_PROBES).max(1);
    let classifier = body.classifier();
    let (mut inside, mut outside) = (0_usize, 0_usize);
    for origin in origins.into_iter().step_by(stride) {
        interrupt::check()?;
        match classifier.classify(plane.to_world(origin) + direction * RAY_START) {
            PointClass::Inside => inside += 1,
            PointClass::Outside => outside += 1,
            PointClass::OnBoundary(_) | PointClass::Undecided => {}
        }
    }
    let entering = match (inside, outside) {
        (0, 0) => return Err(StopError::Straddles),
        (0, _) => true,
        (_, 0) => false,
        _ => return Err(StopError::Straddles),
    };
    let operation = if entering {
        BooleanOperation::Difference
    } else {
        BooleanOperation::Intersection
    };
    let piece = match boolean(tool, body, operation) {
        Ok(piece) => piece,
        Err(BooleanError::Empty) => return Err(StopError::Nothing),
        Err(error) => return Err(error.into()),
    };
    let spans = piece.shell_spans(plane.origin(), direction)?;
    if spans.values().any(|span| span.void) {
        return Err(StopError::Enclosed);
    }
    let kept: BTreeSet<ShellId> = spans
        .iter()
        .filter(|(_, span)| span.least <= RAY_START)
        .map(|(shell, _)| *shell)
        .collect();
    if kept
        .iter()
        .filter_map(|shell| spans.get(shell))
        .any(|span| span.most >= far - RAY_START)
    {
        return Err(StopError::PassesBeside);
    }
    if kept.is_empty() {
        return Err(StopError::Nothing);
    }
    let solid = piece.keeping_shells(&kept).ok_or(StopError::Nothing)?;
    Ok(Stopped { solid, entering })
}
