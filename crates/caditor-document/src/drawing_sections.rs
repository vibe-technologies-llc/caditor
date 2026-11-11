use std::{
    f64::consts::FRAC_PI_4,
    panic::{self, AssertUnwindSafe},
};

use caditor_expression::Dimension;
use caditor_geometry::{Aabb, Plane, Point2, Point3, Vector2};
use caditor_kernel::{
    BooleanError, BooleanOperation, HiddenLineError, HiddenLineView, LINEAR_RESOLUTION,
    MeshQuality, SamplingTolerance, Solid, SweepError, boolean, check_interrupt,
};

use crate::{
    document::FeatureId,
    drawing_sheet::PlacedView,
    drawing_views::{
        DrawingViewError, HatchRegion, SectionLines, ViewLines, lines_seen, scene_extent,
    },
    drawings::{DrawnView, MAX_SECTION_OFFSET, SectionPlace, SectionView},
    section::is_cut_face,
    split::{HalfSpaceError, half_space_solid, heights},
    values::ParameterValues,
};

pub const HATCH_SPACING: f64 = 2.5;
pub const HATCH_ANGLES: [f64; 2] = [FRAC_PI_4, 3.0 * FRAC_PI_4];
const MOST_HATCH_LINES: f64 = 2000.0;
const SECTION_CUT: FeatureId = FeatureId::from_raw(u64::MAX - 64);

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ResolvedPlace {
    Centre,
    Offset(f64),
}

impl SectionPlace {
    pub fn resolve(&self, values: &ParameterValues) -> Result<ResolvedPlace, DrawingViewError> {
        let Self::Offset(offset) = self else {
            return Ok(ResolvedPlace::Centre);
        };
        let millimetres = offset
            .evaluate_as(Dimension::LENGTH, &|id| values.value(id))
            .map_err(DrawingViewError::Offset)?;
        if millimetres.is_finite() && millimetres.abs() <= MAX_SECTION_OFFSET {
            Ok(ResolvedPlace::Offset(millimetres))
        } else {
            Err(DrawingViewError::OffsetOutOfRange)
        }
    }
}

enum Side {
    Removed,
    Whole,
    Cut,
}

fn bodies_centre(solids: &[&Solid]) -> Option<Point3> {
    solids
        .iter()
        .filter_map(|solid| solid.bounding_box())
        .reduce(Aabb::union)
        .map(|bounds| bounds.center())
        .filter(|centre| centre.is_finite())
}

pub fn cutting_plane(
    section: &SectionView,
    place: ResolvedPlace,
    solids: &[&Solid],
) -> Result<Plane, DrawingViewError> {
    let (look, _) = section.look_and_up();
    let axis = section.plane.normal();
    let position = match place {
        ResolvedPlace::Centre => bodies_centre(solids).map_or(0.0, |centre| centre.dot(axis)),
        ResolvedPlace::Offset(offset) => offset,
    };
    Plane::new(axis * position, look)
        .ok_or(DrawingViewError::Lines(HiddenLineError::NoLookDirection))
}

fn side_of(solid: &Solid, cutting: &Plane) -> Side {
    let Some(heights) = heights(solid, cutting, cutting.normal()) else {
        return Side::Removed;
    };
    if heights.high <= LINEAR_RESOLUTION {
        Side::Removed
    } else if heights.low >= -LINEAR_RESOLUTION {
        Side::Whole
    } else {
        Side::Cut
    }
}

fn cut(solid: &Solid, cutting: &Plane) -> Result<Option<Solid>, DrawingViewError> {
    let kept_side = half_space_solid(solid, cutting, false, SECTION_CUT.raw()).map_err(
        |error| match error {
            HalfSpaceError::Sweep(SweepError::Cancelled(_)) => DrawingViewError::Cancelled,
            other => DrawingViewError::CuttingSpace(other),
        },
    )?;
    match boolean(solid, &kept_side, BooleanOperation::Intersection) {
        Ok(kept) => Ok(Some(kept)),
        Err(BooleanError::Empty) => Ok(None),
        Err(BooleanError::Cancelled(_)) => Err(DrawingViewError::Cancelled),
        Err(other) => Err(DrawingViewError::Cut(other)),
    }
}

fn cut_face_edges(
    solid: &Solid,
    frame: &Plane,
    tolerance: &SamplingTolerance,
) -> Vec<(Point2, Point2)> {
    let mut edges = Vec::new();
    let boundary = solid
        .faces()
        .filter(|(_, face)| is_cut_face(face, SECTION_CUT))
        .flat_map(|(_, face)| face.loops())
        .filter_map(|id| solid.face_loop(*id))
        .flat_map(|face_loop| face_loop.coedges())
        .filter_map(|coedge| solid.coedge(*coedge))
        .filter_map(|coedge| solid.edge(coedge.edge()));
    for edge in boundary {
        let points: Vec<Point2> = edge
            .curve()
            .sample(edge.interval(), tolerance)
            .into_iter()
            .map(|sample| frame.to_local(sample.point))
            .collect();
        edges.extend(points.iter().copied().zip(points.iter().skip(1).copied()));
    }
    edges
}

fn sectioned(
    section: &SectionView,
    place: ResolvedPlace,
    solids: &[&Solid],
) -> Result<ViewLines, DrawingViewError> {
    let (look, up) = section.look_and_up();
    let looking = HiddenLineView::new(look, up)?;
    let cutting = cutting_plane(section, place, solids)?;
    let mut whole = Vec::new();
    let mut cut_solids = Vec::new();
    for solid in solids {
        check_interrupt().map_err(|_| DrawingViewError::Cancelled)?;
        match side_of(solid, &cutting) {
            Side::Removed => {}
            Side::Whole => whole.push(*solid),
            Side::Cut => cut_solids.extend(cut(solid, &cutting)?),
        }
    }
    if cut_solids.is_empty() {
        return Err(DrawingViewError::PlaneMisses);
    }
    let shown: Vec<&Solid> = whole.into_iter().chain(cut_solids.iter()).collect();
    let mut lines = lines_seen(DrawnView::Section(section.letter), &looking, &shown)?;
    let tolerance = MeshQuality::SMOOTH.tolerance(scene_extent(&shown));
    let hatching = cut_solids
        .iter()
        .zip(HATCH_ANGLES.iter().cycle())
        .map(|(solid, angle)| HatchRegion {
            edges: cut_face_edges(solid, looking.frame(), &tolerance),
            angle: *angle,
        })
        .filter(|region| !region.edges.is_empty())
        .collect();
    lines.section = Some(SectionLines { cutting, hatching });
    Ok(lines)
}

pub fn section_view(
    section: &SectionView,
    place: ResolvedPlace,
    solids: &[&Solid],
) -> Result<ViewLines, DrawingViewError> {
    if solids.is_empty() {
        return Err(DrawingViewError::NoBodies);
    }
    panic::catch_unwind(AssertUnwindSafe(|| sectioned(section, place, solids)))
        .map_err(|_| DrawingViewError::Panicked)?
}

fn crossings_at(
    edges: &[(Point2, Point2)],
    across: Vector2,
    along: Vector2,
    height: f64,
) -> Vec<f64> {
    let mut crossings: Vec<f64> = edges
        .iter()
        .filter_map(|(start, end)| {
            let (from, to) = (start.dot(across), end.dot(across));
            if (from > height) == (to > height) {
                return None;
            }
            let share = (height - from) / (to - from);
            Some((*start + (*end - *start) * share).dot(along))
        })
        .collect();
    crossings.sort_by(f64::total_cmp);
    crossings
}

pub fn hatch_lines(region: &HatchRegion, placed: &PlacedView) -> Vec<(Point2, Point2)> {
    let along = Vector2::from_angle(region.angle);
    let across = along.perp();
    let edges: Vec<(Point2, Point2)> = region
        .edges
        .iter()
        .map(|(start, end)| (placed.on_sheet(*start), placed.on_sheet(*end)))
        .collect();
    let (low, high) = edges
        .iter()
        .flat_map(|(start, end)| [start.dot(across), end.dot(across)])
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), height| {
            (low.min(height), high.max(height))
        });
    if !(low.is_finite() && high.is_finite()) || high <= low {
        return Vec::new();
    }
    let spacing = HATCH_SPACING.max((high - low) / MOST_HATCH_LINES);
    let first = (low / spacing).ceil() as i64;
    let last = (high / spacing).floor() as i64;
    let mut lines = Vec::new();
    for step in first..=last {
        let height = step as f64 * spacing;
        let crossings = crossings_at(&edges, across, along, height);
        let (pairs, _) = crossings.as_chunks::<2>();
        for [from, to] in pairs {
            lines.push((
                across * height + along * *from,
                across * height + along * *to,
            ));
        }
    }
    lines
}
