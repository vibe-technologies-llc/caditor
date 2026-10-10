use caditor_geometry::{Plane, Point3, Vector3};
use thiserror::Error;

use super::Named;
use crate::{
    bspline::BSpline,
    curve::{BSplineCurve, Curve},
    error::GeometryError,
    sense::Sense,
    surface::{BSplineSurface, PlaneSurface, Surface},
    topology::{BuildError, EdgeId, FaceId, ShellId, Solid, SolidBuilder, VertexId},
};

const CUBIC: usize = 3;
const RULED_KNOTS: [f64; 4] = [0.0, 0.0, 1.0, 1.0];

#[derive(Debug, Clone, PartialEq, Error)]
pub enum LoftError {
    #[error("a lofted curve or surface is invalid: {0}")]
    Geometry(#[from] GeometryError),
    #[error("the lofted solid is invalid: {0}")]
    Solid(#[from] BuildError),
    #[error("a run of the blend has fewer than four sections")]
    TooFewSections,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Outline {
    pub chord: [Point3; 2],
    pub apex: Point3,
}

impl Outline {
    fn corners(&self) -> [Point3; 3] {
        [self.chord[0], self.chord[1], self.apex]
    }

    fn centroid(&self) -> Point3 {
        (self.chord[0] + self.chord[1] + self.apex) / 3.0
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Segment {
    pub params: Vec<f64>,
    pub outlines: Vec<Outline>,
    pub name: Named,
}

pub(super) struct Loft<'a> {
    pub segments: &'a [Segment],
    pub closed: bool,
    pub sides: [Named; 2],
    pub caps: [Named; 2],
}

pub(super) fn rows(segment: &Segment) -> Result<[BSplineCurve; 3], LoftError> {
    if segment.outlines.len() <= CUBIC || segment.params.len() != segment.outlines.len() {
        return Err(LoftError::TooFewSections);
    }
    let row = |pick: fn(&Outline) -> Point3| -> Result<BSplineCurve, LoftError> {
        let points: Vec<Point3> = segment.outlines.iter().map(pick).collect();
        Ok(BSpline::interpolating_at(CUBIC, &segment.params, &points)?)
    };
    Ok([
        row(|outline| outline.chord[0])?,
        row(|outline| outline.chord[1])?,
        row(|outline| outline.apex)?,
    ])
}

pub(super) fn ruled(bottom: &BSplineCurve, top: &BSplineCurve) -> Result<Surface, LoftError> {
    let columns = bottom.control_points().len();
    let points: Vec<Point3> = bottom
        .control_points()
        .iter()
        .chain(top.control_points())
        .copied()
        .collect();
    Ok(BSplineSurface::new(
        CUBIC,
        1,
        bottom.knots().to_vec(),
        RULED_KNOTS.to_vec(),
        columns,
        points,
        None,
    )?
    .into())
}

struct Joint {
    vertices: [VertexId; 3],
    edges: [EdgeId; 3],
}

fn joint(builder: &mut SolidBuilder, outline: &Outline) -> Result<Joint, LoftError> {
    let mut vertices = Vec::with_capacity(3);
    for corner in outline.corners() {
        vertices.push(builder.vertex(corner)?);
    }
    let &[a, b, p] = vertices.as_slice() else {
        return Err(LoftError::TooFewSections);
    };
    Ok(Joint {
        vertices: [a, b, p],
        edges: [
            builder.line_edge(a, b)?,
            builder.line_edge(b, p)?,
            builder.line_edge(p, a)?,
        ],
    })
}

fn named_face(
    builder: &mut SolidBuilder,
    shell: ShellId,
    surface: Surface,
    sense: Sense,
    name: Named,
) -> Result<FaceId, LoftError> {
    let face = builder.face(shell, surface, sense)?;
    builder.set_face_name(face, name.name)?;
    if let Some(origin) = name.origin {
        builder.set_face_origin(face, origin)?;
    }
    Ok(face)
}

fn newell(points: &[Point3]) -> Vector3 {
    points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .map(|(a, b)| (*a).cross(*b))
        .sum()
}

fn planar_face(
    builder: &mut SolidBuilder,
    shell: ShellId,
    corners: &[(EdgeId, Sense, Point3)],
    interior: Point3,
    name: Named,
) -> Result<(), LoftError> {
    let points: Vec<Point3> = corners.iter().map(|(_, _, point)| *point).collect();
    let normal = newell(&points)
        .try_normalize()
        .ok_or(GeometryError::ZeroDirection)?;
    let first = *points.first().ok_or(LoftError::TooFewSections)?;
    let centroid = points.iter().copied().sum::<Point3>() / points.len() as f64;
    let outward = (centroid - interior).dot(normal) > 0.0;
    let normal = if outward { normal } else { -normal };
    let second = *points.get(1).ok_or(LoftError::TooFewSections)?;
    let frame =
        Plane::with_x_axis(first, normal, second - first).ok_or(GeometryError::ZeroDirection)?;
    let face = named_face(
        builder,
        shell,
        PlaneSurface::new(frame)?.into(),
        Sense::Same,
        name,
    )?;
    let coedges: Vec<(EdgeId, Sense)> = if outward {
        corners
            .iter()
            .map(|(edge, sense, _)| (*edge, *sense))
            .collect()
    } else {
        corners
            .iter()
            .rev()
            .map(|(edge, sense, _)| (*edge, sense.reversed()))
            .collect()
    };
    builder.add_loop(face, &coedges)?;
    Ok(())
}

fn close_end(
    builder: &mut SolidBuilder,
    shell: ShellId,
    at: &Joint,
    outline: &Outline,
    into_tool: Vector3,
    cap: Named,
) -> Result<(), LoftError> {
    let reach = outline.chord[0].distance(outline.apex);
    let interior = outline.centroid() + into_tool * reach;
    let [ab, bp, pa] = at.edges;
    let [a, b, p] = outline.corners();
    planar_face(
        builder,
        shell,
        &[
            (ab, Sense::Same, a),
            (bp, Sense::Same, b),
            (pa, Sense::Same, p),
        ],
        interior,
        cap,
    )
}

fn outline_normal(outline: &Outline) -> Option<Vector3> {
    newell(&outline.corners()).try_normalize()
}

pub(super) fn loft(loft: &Loft<'_>) -> Result<Solid, LoftError> {
    let segments = loft.segments;
    let first = segments.first().ok_or(LoftError::TooFewSections)?;
    let last = segments.last().ok_or(LoftError::TooFewSections)?;
    let start_outline = *first.outlines.first().ok_or(LoftError::TooFewSections)?;
    let end_outline = *last.outlines.last().ok_or(LoftError::TooFewSections)?;
    let mut builder = SolidBuilder::new();
    let shell = builder.shell()?;
    let mut joints = Vec::with_capacity(segments.len() + 1);
    for segment in segments {
        let outline = segment.outlines.first().ok_or(LoftError::TooFewSections)?;
        joints.push(joint(&mut builder, outline)?);
    }
    if !loft.closed {
        joints.push(joint(&mut builder, &end_outline)?);
    }
    let count = joints.len();
    let along = segments
        .first()
        .and_then(|segment| {
            let next = segment.outlines.get(1)?;
            (next.centroid() - start_outline.centroid()).try_normalize()
        })
        .ok_or(GeometryError::ZeroDirection)?;
    let winding = outline_normal(&start_outline).ok_or(GeometryError::ZeroDirection)?;
    let sense = if winding.dot(along) < 0.0 {
        Sense::Same
    } else {
        Sense::Reversed
    };
    for (index, segment) in segments.iter().enumerate() {
        let (Some(start), Some(end)) = (joints.get(index), joints.get((index + 1) % count)) else {
            return Err(LoftError::TooFewSections);
        };
        let [a_row, b_row, p_row] = rows(segment)?;
        let domain = a_row.domain();
        let mut row_edges = Vec::with_capacity(3);
        for (row, (from, to)) in [&a_row, &b_row, &p_row]
            .into_iter()
            .zip(start.vertices.into_iter().zip(end.vertices))
        {
            row_edges.push(builder.edge(Curve::BSpline(row.clone()), domain, from, to)?);
        }
        let &[a_edge, b_edge, p_edge] = row_edges.as_slice() else {
            return Err(LoftError::TooFewSections);
        };
        let faces = [
            (&a_row, &b_row, a_edge, b_edge, 0, segment.name),
            (&b_row, &p_row, b_edge, p_edge, 1, loft.sides[1]),
            (&p_row, &a_row, p_edge, a_edge, 2, loft.sides[0]),
        ];
        for (bottom, top, bottom_edge, top_edge, joint_index, name) in faces {
            let (Some(start_joint), Some(end_joint)) = (
                start.edges.get(joint_index).copied(),
                end.edges.get(joint_index).copied(),
            ) else {
                return Err(LoftError::TooFewSections);
            };
            let face = named_face(&mut builder, shell, ruled(bottom, top)?, sense, name)?;
            let coedges = match sense {
                Sense::Same => [
                    (bottom_edge, Sense::Same),
                    (end_joint, Sense::Same),
                    (top_edge, Sense::Reversed),
                    (start_joint, Sense::Reversed),
                ],
                Sense::Reversed => [
                    (start_joint, Sense::Same),
                    (top_edge, Sense::Same),
                    (end_joint, Sense::Reversed),
                    (bottom_edge, Sense::Reversed),
                ],
            };
            builder.add_loop(face, &coedges)?;
        }
    }
    if !loft.closed {
        let (Some(start), Some(end)) = (joints.first(), joints.last()) else {
            return Err(LoftError::TooFewSections);
        };
        let into_end = last
            .outlines
            .iter()
            .rev()
            .nth(1)
            .and_then(|before| (before.centroid() - end_outline.centroid()).try_normalize())
            .ok_or(GeometryError::ZeroDirection)?;
        close_end(
            &mut builder,
            shell,
            start,
            &start_outline,
            along,
            loft.caps[0],
        )?;
        close_end(
            &mut builder,
            shell,
            end,
            &end_outline,
            into_end,
            loft.caps[1],
        )?;
    }
    Ok(builder.build()?)
}
