use std::{collections::BTreeSet, f64::consts::FRAC_PI_2};

use caditor_geometry::{Aabb2, Plane, Point2, Point3};

use crate::{
    build::{
        LinearExtent, SweepError,
        extrude::{Level, extrude, offset_plane, settled},
        lift,
        plan::{Plan, PlanCoedge, PlanFace, outward_sense},
        taper_bent::{Rung, offset_surface},
    },
    curve::{Curve, IntersectionCurve},
    error::GeometryError,
    interrupt,
    naming::{EdgeName, FaceName, FaceOrigin},
    profile::{
        Course, OffsetFailure, Piece, Region, Strand, is_smooth_turn, offset_courses, outline_of,
        polygons_cross, signed_area,
    },
    sense::Sense,
    surface::{Cone, PlaneSurface, Surface},
    tolerance::{LINEAR_RESOLUTION, MAX_SIZE},
    topology::Solid,
};

const STRAIGHT_TAPER: f64 = 1e-5;
pub const MAX_TAPER_DEGREES: f64 = 89.0;
pub const MIN_FLANK_DEGREES: f64 = 0.1;
const JOINT_SAMPLES: usize = 12;
const RELATIVE_TOLERANCE: f64 = 1e-7;

fn is_smooth(before: &Course, after: &Course) -> bool {
    let (leaving, entering) = (before.end_tangent(), after.start_tangent());
    is_smooth_turn(leaving, entering) && leaving.dot(entering) > 0.0
}

struct Taper {
    reference: f64,
    rate: f64,
}

impl Taper {
    fn inset(&self, height: f64) -> f64 {
        self.rate * (height - self.reference).abs()
    }
}

struct Sweep<'a> {
    plane: &'a Plane,
    levels: Vec<f64>,
    taper: Taper,
    feature: u64,
    end_segment: usize,
    tolerance: f64,
}

impl Sweep<'_> {
    fn point(&self, point: Point2, height: f64) -> Point3 {
        self.plane.to_world(point) + self.plane.normal() * height
    }

    fn offset(&self, profile_loop: &TaperedLoop, height: f64) -> Result<Vec<Course>, SweepError> {
        self.offset_near(profile_loop, height, None)
    }

    fn offset_near(
        &self,
        profile_loop: &TaperedLoop,
        height: f64,
        aims: Option<&[Course]>,
    ) -> Result<Vec<Course>, SweepError> {
        offset_courses(
            &profile_loop.courses,
            true,
            self.taper.inset(height),
            aims,
            self.tolerance,
        )
        .map_err(|failure| profile_loop.closes(failure))
    }

    fn going(&self, segment: usize) -> f64 {
        let low = self.levels.get(segment).copied().unwrap_or_default();
        let high = self.levels.get(segment + 1).copied().unwrap_or_default();
        if 0.5 * (low + high) >= self.taper.reference {
            1.0
        } else {
            -1.0
        }
    }

    fn surface(
        &self,
        course: &Course,
        going: f64,
        segment: usize,
        trimmed: [&Course; 2],
    ) -> Result<Surface, SweepError> {
        match course {
            Course::Plain(strand) => self.strand_surface(strand, going),
            Course::Bent { bend, .. } => {
                let rung = |index: usize| {
                    let height = self.levels.get(index).copied().unwrap_or_default();
                    Rung {
                        height,
                        inset: self.taper.inset(height),
                    }
                };
                let rungs = [rung(segment), rung(segment + 1)];
                let [low, high] = trimmed.map(Course::overrun);
                let overrun = [low[0].max(high[0]), low[1].max(high[1])];
                offset_surface(self.plane, bend, rungs, overrun)
            }
            Course::Trimmed(_) => Err(SweepError::Unassembled),
        }
    }

    fn strand_surface(&self, strand: &Strand, going: f64) -> Result<Surface, SweepError> {
        let normal = self.plane.normal();
        let zero_direction = SweepError::Geometry(GeometryError::ZeroDirection);
        match *strand {
            Strand::Line { start, .. } => {
                let tangent = strand.start_tangent();
                let along = lift(self.plane, tangent);
                let rising = normal + lift(self.plane, tangent.perp()) * (self.taper.rate * going);
                let frame = Plane::from_frame(
                    self.point(start, self.taper.reference),
                    along.cross(rising),
                    along,
                )
                .ok_or(zero_direction)?;
                Ok(PlaneSurface::new(frame)?.into())
            }
            Strand::Arc {
                center,
                radius,
                sweep,
                ..
            } => {
                let half_angle = (-sweep.signum() * self.taper.rate * going).atan();
                let frame = Plane::from_frame(
                    self.point(center, self.taper.reference),
                    normal,
                    self.plane.x_axis(),
                )
                .ok_or(zero_direction)?;
                Ok(Cone::new(frame, radius, half_angle)?.into())
            }
        }
    }
}

struct TaperedLoop {
    pieces: Vec<Piece>,
    courses: Vec<Course>,
}

impl TaperedLoop {
    fn closes(&self, failure: OffsetFailure) -> SweepError {
        let entities: BTreeSet<u64> = failure
            .strands()
            .into_iter()
            .filter_map(|index| self.pieces.get(index).map(Piece::entity))
            .collect();
        SweepError::TaperCloses {
            entities: entities.into_iter().collect(),
        }
    }

    fn entities(&self) -> Vec<u64> {
        let entities: BTreeSet<u64> = self.pieces.iter().map(Piece::entity).collect();
        entities.into_iter().collect()
    }
}

fn tapered_loops(regions: &[Region]) -> Result<Vec<Vec<TaperedLoop>>, SweepError> {
    let mut unsupported = BTreeSet::new();
    let loops: Vec<Vec<TaperedLoop>> = regions
        .iter()
        .map(|region| {
            region
                .loops()
                .map(|profile_loop| {
                    let courses: Vec<Course> =
                        profile_loop.pieces().iter().map(Course::of_piece).collect();
                    if let [Course::Bent { .. }] = courses.as_slice() {
                        unsupported.extend(profile_loop.pieces().iter().map(Piece::entity));
                    }
                    TaperedLoop {
                        pieces: profile_loop.pieces().to_vec(),
                        courses,
                    }
                })
                .collect()
        })
        .collect();
    if unsupported.is_empty() {
        Ok(loops)
    } else {
        Err(SweepError::TaperedUnsupportedCurve {
            entities: unsupported.into_iter().collect(),
        })
    }
}

fn levels(start: f64, end: f64) -> (Vec<f64>, f64) {
    let (low, high) = (start.min(end), start.max(end));
    let reference = 0.0_f64.clamp(low, high);
    let reference = if reference - low <= LINEAR_RESOLUTION {
        low
    } else if high - reference <= LINEAR_RESOLUTION {
        high
    } else {
        reference
    };
    let mut levels = vec![low];
    if reference > low && reference < high {
        levels.push(reference);
    }
    levels.push(high);
    (levels, reference)
}

pub fn extrude_tapered(
    plane: &Plane,
    regions: &[Region],
    extent: LinearExtent,
    angle: f64,
    feature: u64,
) -> Result<Solid, SweepError> {
    if !angle.is_finite() {
        return Err(SweepError::NonFinite);
    }
    if angle.abs() < STRAIGHT_TAPER {
        return extrude(plane, regions, extent, feature);
    }
    if angle.abs().to_degrees() >= MAX_TAPER_DEGREES {
        return Err(SweepError::TaperTooSteep);
    }
    tapered(plane, regions, extent, angle.tan(), feature)
}

pub fn extrude_flared(
    plane: &Plane,
    regions: &[Region],
    extent: LinearExtent,
    flank: f64,
    feature: u64,
) -> Result<Solid, SweepError> {
    if !flank.is_finite() {
        return Err(SweepError::NonFinite);
    }
    if flank.to_degrees() < MIN_FLANK_DEGREES || flank >= FRAC_PI_2 {
        return Err(SweepError::FlankOutOfRange);
    }
    tapered(plane, regions, extent, flank.tan().recip(), feature)
}

fn tapered(
    plane: &Plane,
    regions: &[Region],
    extent: LinearExtent,
    rate: f64,
    feature: u64,
) -> Result<Solid, SweepError> {
    if regions.is_empty() {
        return Err(SweepError::NoRegions);
    }
    let start = settled(Level::of(plane, extent.start())?, regions).flat_height();
    let end = settled(Level::of(plane, extent.end())?, regions).flat_height();
    let (Some(start), Some(end)) = (start, end) else {
        return Err(SweepError::TaperedTiltedEnd);
    };
    if start.abs().max(end.abs()) > MAX_SIZE {
        return Err(SweepError::TooLong);
    }
    if (end - start).abs() <= LINEAR_RESOLUTION {
        return Err(SweepError::ZeroLength);
    }
    let loops = tapered_loops(regions)?;
    let size = regions
        .iter()
        .filter_map(Region::bounds)
        .reduce(Aabb2::union)
        .map_or(1.0, |bounds| {
            bounds.size().length() + bounds.min().abs().max(bounds.max().abs()).max_element()
        });
    let (levels, reference) = levels(start, end);
    let start_is_low = start < end;
    let segments = levels.len() - 1;
    let sweep = Sweep {
        plane,
        levels,
        taper: Taper { reference, rate },
        feature,
        end_segment: if start_is_low { segments - 1 } else { 0 },
        tolerance: (size * RELATIVE_TOLERANCE).max(LINEAR_RESOLUTION),
    };
    let offsets: Vec<Vec<Vec<Vec<Course>>>> = loops
        .iter()
        .map(|region_loops| {
            region_loops
                .iter()
                .map(|profile_loop| {
                    sweep
                        .levels
                        .iter()
                        .map(|height| sweep.offset(profile_loop, *height))
                        .collect::<Result<Vec<_>, _>>()
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .collect::<Result<_, _>>()?;
    check_caps(&sweep, &loops, &offsets)?;
    let mut plan = Plan::default();
    for ((region, region_loops), region_offsets) in regions.iter().zip(&loops).zip(&offsets) {
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
        let (low_cap, high_cap) = if start_is_low {
            (start_cap, end_cap)
        } else {
            (end_cap, start_cap)
        };
        let mut bottom_loops = Vec::new();
        let mut top_loops = Vec::new();
        for (profile_loop, at_levels) in region_loops.iter().zip(region_offsets) {
            let (bottom, top) = tapered_loop(
                &mut plan,
                &sweep,
                profile_loop,
                at_levels,
                (low_cap.0, high_cap.0),
            )?;
            bottom_loops.push(bottom);
            top_loops.push(top);
        }
        let (low, high) = (
            sweep.levels.first().copied().unwrap_or_default(),
            sweep.levels.last().copied().unwrap_or_default(),
        );
        plan.face(PlanFace {
            surface: PlaneSurface::new(offset_plane(plane, low)?.flipped())?.into(),
            sense: Sense::Same,
            name: low_cap.0,
            origin: Some(low_cap.1),
            loops: bottom_loops,
        });
        plan.face(PlanFace {
            surface: PlaneSurface::new(offset_plane(plane, high)?)?.into(),
            sense: Sense::Same,
            name: high_cap.0,
            origin: Some(high_cap.1),
            loops: top_loops,
        });
    }
    Ok(plan.build()?)
}

fn check_caps(
    sweep: &Sweep<'_>,
    loops: &[Vec<TaperedLoop>],
    offsets: &[Vec<Vec<Vec<Course>>>],
) -> Result<(), SweepError> {
    let last = sweep.levels.len() - 1;
    for level in [0, last] {
        let mut polygons = Vec::new();
        for (region_loops, region_offsets) in loops.iter().zip(offsets) {
            for (profile_loop, at_levels) in region_loops.iter().zip(region_offsets) {
                let Some(courses) = at_levels.get(level) else {
                    continue;
                };
                let moved = outline_of(courses);
                let original = signed_area(&outline_of(&profile_loop.courses));
                let area = signed_area(&moved);
                let tiny = sweep.tolerance * sweep.tolerance;
                if area.abs() <= tiny || area.signum() != original.signum() {
                    return Err(SweepError::TaperCloses {
                        entities: profile_loop.entities(),
                    });
                }
                polygons.push(moved);
            }
        }
        if polygons_cross(&polygons) {
            return Err(SweepError::TaperCloses {
                entities: Vec::new(),
            });
        }
    }
    Ok(())
}

fn tapered_loop(
    plan: &mut Plan,
    sweep: &Sweep<'_>,
    profile_loop: &TaperedLoop,
    at_levels: &[Vec<Course>],
    (low_cap, high_cap): (FaceName, FaceName),
) -> Result<(Vec<PlanCoedge>, Vec<PlanCoedge>), SweepError> {
    let count = profile_loop.courses.len();
    let segments = sweep.levels.len() - 1;
    let wrap = |index: usize| index % count.max(1);
    let names: Vec<Vec<FaceName>> = (0..segments)
        .map(|segment| {
            profile_loop
                .pieces
                .iter()
                .map(|piece| {
                    if segment == sweep.end_segment {
                        FaceName::side(sweep.feature, piece.id())
                    } else {
                        FaceName::side_behind(sweep.feature, piece.id())
                    }
                })
                .collect()
        })
        .collect();
    let name = |segment: usize, index: usize| {
        names
            .get(segment)
            .and_then(|row| row.get(wrap(index)))
            .copied()
            .unwrap_or_default()
    };
    let vertices: Vec<Vec<usize>> = at_levels
        .iter()
        .zip(&sweep.levels)
        .map(|(courses, height)| {
            courses
                .iter()
                .map(|course| plan.vertex(sweep.point(course.start(), *height)))
                .collect()
        })
        .collect();
    let vertex = |level: usize, index: usize| {
        vertices
            .get(level)
            .and_then(|row| row.get(wrap(index)))
            .copied()
            .ok_or(SweepError::Unassembled)
    };
    let mut rings: Vec<Vec<(usize, Sense)>> = Vec::with_capacity(at_levels.len());
    for (level, (courses, height)) in at_levels.iter().zip(&sweep.levels).enumerate() {
        let level_plane = offset_plane(sweep.plane, *height)?;
        let mut ring = Vec::with_capacity(count);
        for (index, course) in courses.iter().enumerate() {
            let (curve, range, reversed) = course.curve()?;
            let (from, to) = (vertex(level, index)?, vertex(level, index + 1)?);
            let edge_name = if level == 0 {
                EdgeName::between(name(0, index), low_cap)
            } else if level == segments {
                EdgeName::between(name(segments - 1, index), high_cap)
            } else {
                EdgeName::between(name(level - 1, index), name(level, index))
            };
            let edge = plan.edge(
                curve.on_plane(&level_plane)?,
                range,
                if reversed { (to, from) } else { (from, to) },
                edge_name,
            );
            ring.push((
                edge,
                if reversed {
                    Sense::Reversed
                } else {
                    Sense::Same
                },
            ));
        }
        rings.push(ring);
    }
    let ring = |level: usize, index: usize| {
        rings
            .get(level)
            .and_then(|row| row.get(index))
            .copied()
            .ok_or(SweepError::Unassembled)
    };
    for segment in 0..segments {
        interrupt::check()?;
        let going = sweep.going(segment);
        let (Some(low_courses), Some(high_courses)) =
            (at_levels.get(segment), at_levels.get(segment + 1))
        else {
            return Err(SweepError::Unassembled);
        };
        let surfaces: Vec<Surface> = profile_loop
            .courses
            .iter()
            .enumerate()
            .map(|(index, course)| {
                let (Some(low), Some(high)) = (low_courses.get(index), high_courses.get(index))
                else {
                    return Err(SweepError::Unassembled);
                };
                sweep.surface(course, going, segment, [low, high])
            })
            .collect::<Result<_, _>>()?;
        let samples = joint_samples(sweep, profile_loop, at_levels, segment)?;
        let mut verticals = Vec::with_capacity(count);
        for joint in 0..count {
            let before = wrap(joint + count - 1);
            let edge_name = EdgeName::between(name(segment, before), name(segment, joint));
            let ends = (vertex(segment, joint)?, vertex(segment + 1, joint)?);
            let (Some(leaving), Some(entering)) = (
                profile_loop.courses.get(before),
                profile_loop.courses.get(joint),
            ) else {
                return Err(SweepError::Unassembled);
            };
            let straight = count == 1
                || is_smooth(leaving, entering)
                || (leaving.is_line() && entering.is_line());
            let edge = if straight {
                plan.line(ends.0, ends.1, edge_name)?
            } else {
                let points: Vec<Point3> = samples
                    .iter()
                    .filter_map(|(height, courses)| {
                        courses
                            .get(joint)
                            .map(|course| sweep.point(course.start(), *height))
                    })
                    .collect();
                let (Some(first), Some(second)) = (surfaces.get(before), surfaces.get(joint))
                else {
                    return Err(SweepError::Unassembled);
                };
                let curve =
                    IntersectionCurve::through([first.clone(), second.clone()], &points, false)
                        .ok_or(SweepError::Unassembled)?;
                let interval = curve.domain();
                plan.edge(Curve::Intersection(curve), interval, ends, edge_name)
            };
            verticals.push(edge);
        }
        let (low_strands, high_strands) = (low_courses, high_courses);
        let (low_height, high_height) = (
            sweep.levels.get(segment).copied().unwrap_or_default(),
            sweep.levels.get(segment + 1).copied().unwrap_or_default(),
        );
        for (index, (piece, surface)) in profile_loop.pieces.iter().zip(surfaces).enumerate() {
            let (bottom, travel) = ring(segment, index)?;
            let (top, top_travel) = ring(segment + 1, index)?;
            let entering = verticals
                .get(index)
                .copied()
                .ok_or(SweepError::Unassembled)?;
            let leaving = verticals
                .get(wrap(index + 1))
                .copied()
                .ok_or(SweepError::Unassembled)?;
            let (Some(original), Some(low_strand), Some(high_strand)) = (
                profile_loop.courses.get(index),
                low_strands.get(index),
                high_strands.get(index),
            ) else {
                return Err(SweepError::Unassembled);
            };
            let (_, tangent) = original.middle();
            let probe = sweep
                .point(low_strand.middle().0, low_height)
                .midpoint(sweep.point(high_strand.middle().0, high_height));
            let sense = outward_sense(&surface, probe, lift(sweep.plane, -tangent.perp()), None);
            plan.face(PlanFace {
                surface,
                sense,
                name: name(segment, index),
                origin: Some(FaceOrigin::Side {
                    feature: sweep.feature,
                    entity: piece.entity(),
                }),
                loops: vec![vec![
                    PlanCoedge::new(bottom, travel),
                    PlanCoedge::new(leaving, Sense::Same),
                    PlanCoedge::new(top, top_travel.reversed()),
                    PlanCoedge::new(entering, Sense::Reversed),
                ]],
            });
        }
    }
    let mut bottom_loop: Vec<PlanCoedge> = (0..count)
        .map(|index| ring(0, index).map(|(edge, travel)| PlanCoedge::new(edge, travel.reversed())))
        .collect::<Result<_, _>>()?;
    bottom_loop.reverse();
    let top_loop = (0..count)
        .map(|index| ring(segments, index).map(|(edge, travel)| PlanCoedge::new(edge, travel)))
        .collect::<Result<_, _>>()?;
    Ok((bottom_loop, top_loop))
}

fn joint_samples(
    sweep: &Sweep<'_>,
    profile_loop: &TaperedLoop,
    at_levels: &[Vec<Course>],
    segment: usize,
) -> Result<Vec<(f64, Vec<Course>)>, SweepError> {
    let count = profile_loop.courses.len();
    let curved = (0..count).any(|joint| {
        let before = (joint + count - 1) % count;
        match (
            profile_loop.courses.get(before),
            profile_loop.courses.get(joint),
        ) {
            (Some(leaving), Some(entering)) => {
                count > 1
                    && !is_smooth(leaving, entering)
                    && !(leaving.is_line() && entering.is_line())
            }
            _ => false,
        }
    });
    if !curved {
        return Ok(Vec::new());
    }
    let (Some(low), Some(high), Some(first), Some(last)) = (
        sweep.levels.get(segment).copied(),
        sweep.levels.get(segment + 1).copied(),
        at_levels.get(segment),
        at_levels.get(segment + 1),
    ) else {
        return Err(SweepError::Unassembled);
    };
    let mut samples = Vec::with_capacity(JOINT_SAMPLES + 1);
    samples.push((low, first.clone()));
    for step in 1..JOINT_SAMPLES {
        let height = low + (high - low) * step as f64 / JOINT_SAMPLES as f64;
        let aims = samples.last().map(|(_, courses)| courses.as_slice());
        let courses = sweep.offset_near(profile_loop, height, aims)?;
        samples.push((height, courses));
    }
    samples.push((high, last.clone()));
    Ok(samples)
}
