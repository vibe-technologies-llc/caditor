mod rings;
mod skin;
#[cfg(test)]
mod tests;

use caditor_geometry::{Plane, Point3};
use thiserror::Error;

use self::{
    rings::{Ring, rings},
    skin::{Beziers, compatible, ruled, skinned},
};
use super::{
    SweepError,
    plan::{Plan, PlanCoedge, PlanFace, outward_sense},
};
use crate::{
    curve::Curve,
    error::GeometryError,
    interrupt::{self, Interrupted},
    naming::{EdgeName, FaceName, FaceOrigin},
    profile::Region,
    sense::Sense,
    surface::{BSplineSurface, PlaneSurface, Surface, SurfaceSide},
    tolerance::{LINEAR_RESOLUTION, MAX_SIZE},
    topology::{CrossingCheck, Solid},
};

pub const MAX_LOFT_PROFILES: usize = 64;
const MAX_SPANS: usize = 128;
const MAX_SEGMENTS: usize = 512;
const LEAST_RISE: f64 = 1e-2;
const RELATIVE_TOLERANCE: f64 = 1e-7;

#[derive(Debug, Clone, PartialEq)]
pub struct LoftProfile {
    pub plane: Plane,
    pub region: Region,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum LoftSkin {
    Ruled,
    #[default]
    Smooth,
}

#[derive(Debug, Clone, PartialEq, Error)]
pub enum LoftError {
    #[error("a loft needs at least two profiles, or a profile and a point")]
    TooFewProfiles,
    #[error("a loft takes at most {MAX_LOFT_PROFILES} profiles")]
    TooManyProfiles,
    #[error("profile {profile} has holes")]
    Holes { profile: usize },
    #[error("profile {profile} lies along the direction the loft runs")]
    AlongProfile { profile: usize },
    #[error("profiles {profiles:?} lie in the same place")]
    Coincident { profiles: [usize; 2] },
    #[error("the loft reaches farther than {MAX_SIZE} mm")]
    TooLarge,
    #[error("the profiles are too intricate to loft")]
    TooIntricate,
    #[error("the lofted solid runs into itself")]
    CrossesItself,
    #[error("the lofted geometry cannot be built: {0}")]
    Geometry(GeometryError),
    #[error(transparent)]
    Build(SweepError),
    #[error(transparent)]
    Cancelled(Interrupted),
}

impl From<GeometryError> for LoftError {
    fn from(error: GeometryError) -> Self {
        Self::Geometry(error)
    }
}

impl From<Interrupted> for LoftError {
    fn from(interrupted: Interrupted) -> Self {
        Self::Cancelled(interrupted)
    }
}

impl From<SweepError> for LoftError {
    fn from(error: SweepError) -> Self {
        match error {
            SweepError::Cancelled(interrupted) => Self::Cancelled(interrupted),
            SweepError::Geometry(error) => Self::Geometry(error),
            other => Self::Build(other),
        }
    }
}

struct Leg {
    first: usize,
    last: usize,
    surfaces: Vec<BSplineSurface>,
}

struct Loft<'a> {
    rings: Vec<Ring<'a>>,
    apex: Option<Point3>,
    count: usize,
    feature: u64,
    start_cap: FaceName,
    end_cap: FaceName,
}

pub fn loft(
    profiles: &[LoftProfile],
    apex: Option<Point3>,
    skin: LoftSkin,
    feature: u64,
) -> Result<Solid, LoftError> {
    let rows = profiles.len() + usize::from(apex.is_some());
    if profiles.is_empty() || rows < 2 {
        return Err(LoftError::TooFewProfiles);
    }
    if profiles.len() > MAX_LOFT_PROFILES {
        return Err(LoftError::TooManyProfiles);
    }
    let size = reach(profiles, apex)?;
    let tolerance = (size * RELATIVE_TOLERANCE).max(LINEAR_RESOLUTION);
    let rings = rings(profiles, apex, tolerance)?;
    let count = rings.first().map_or(0, |ring| ring.spans.len());
    let (Some(first), Some(last)) = (profiles.first(), profiles.last()) else {
        return Err(LoftError::TooFewProfiles);
    };
    let loft = Loft {
        rings,
        apex,
        count,
        feature,
        start_cap: FaceName::start_cap(feature, first.region.key()),
        end_cap: FaceName::end_cap(feature, last.region.key()),
    };
    let solid = loft.build(skin)?;
    if let CrossingCheck::Crossing(_) = solid.find_crossing()? {
        return Err(LoftError::CrossesItself);
    }
    Ok(solid)
}

fn reach(profiles: &[LoftProfile], apex: Option<Point3>) -> Result<f64, LoftError> {
    let mut points: Vec<Point3> = apex.into_iter().collect();
    for profile in profiles {
        let bounds = profile.region.bounds().ok_or(LoftError::TooFewProfiles)?;
        points.push(profile.plane.to_world(bounds.min()));
        points.push(profile.plane.to_world(bounds.max()));
    }
    let farthest = points
        .iter()
        .map(|point| point.abs().max_element())
        .fold(0.0, f64::max);
    if !farthest.is_finite() || farthest > MAX_SIZE {
        return Err(LoftError::TooLarge);
    }
    Ok(farthest.max(1.0))
}

fn degenerate() -> LoftError {
    GeometryError::ZeroDirection.into()
}

impl Loft<'_> {
    fn rows(&self) -> usize {
        self.rings.len() + usize::from(self.apex.is_some())
    }

    fn columns(&self) -> Result<Vec<Vec<Beziers>>, LoftError> {
        let mut columns = Vec::with_capacity(self.count);
        for column in 0..self.count {
            interrupt::check()?;
            let mut curves = self
                .rings
                .iter()
                .map(|ring| {
                    ring.span_spline(column)
                        .and_then(|spline| Beziers::of(&spline))
                })
                .collect::<Result<Vec<_>, _>>()?;
            if let Some(apex) = self.apex {
                curves.push(Beziers::point(apex, 1));
            }
            let matched = compatible(&curves);
            if matched
                .first()
                .is_some_and(|curve| curve.breaks().len() > MAX_SEGMENTS)
            {
                return Err(LoftError::TooIntricate);
            }
            columns.push(matched);
        }
        Ok(columns)
    }

    fn stations(&self) -> Result<Vec<f64>, LoftError> {
        let centres: Vec<Point3> = self
            .rings
            .iter()
            .map(|ring| ring.centre)
            .chain(self.apex)
            .collect();
        let mut travelled = 0.0;
        let mut stations = vec![0.0];
        for pair in centres.windows(2) {
            if let [from, to] = pair {
                travelled += from.distance(*to);
                stations.push(travelled);
            }
        }
        if !(travelled.is_finite() && travelled > 0.0) {
            return Err(degenerate());
        }
        Ok(stations.into_iter().map(|at| at / travelled).collect())
    }

    fn legs(&self, skin: LoftSkin) -> Result<Vec<Leg>, LoftError> {
        let columns = self.columns()?;
        let rows = self.rows();
        match skin {
            LoftSkin::Smooth if rows > 2 => {
                let stations = self.stations()?;
                let surfaces = columns
                    .iter()
                    .map(|curves| {
                        interrupt::check()?;
                        Ok(skinned(curves, &stations)?)
                    })
                    .collect::<Result<Vec<_>, LoftError>>()?;
                Ok(vec![Leg {
                    first: 0,
                    last: rows - 1,
                    surfaces,
                }])
            }
            LoftSkin::Smooth | LoftSkin::Ruled => (0..rows - 1)
                .map(|row| {
                    let surfaces = columns
                        .iter()
                        .map(|curves| {
                            interrupt::check()?;
                            let (Some(bottom), Some(top)) = (curves.get(row), curves.get(row + 1))
                            else {
                                return Err(degenerate());
                            };
                            Ok(ruled(bottom, top)?)
                        })
                        .collect::<Result<Vec<_>, LoftError>>()?;
                    Ok(Leg {
                        first: row,
                        last: row + 1,
                        surfaces,
                    })
                })
                .collect(),
        }
    }

    fn side(&self, leg: usize, column: usize) -> Result<FaceName, LoftError> {
        let ring = self.rings.first().ok_or_else(degenerate)?;
        let span = ring
            .spans
            .get(column % self.count.max(1))
            .ok_or_else(degenerate)?;
        Ok(FaceName::lofted(
            self.feature,
            span.piece.id(),
            span.part,
            u32::try_from(leg).unwrap_or(u32::MAX),
        ))
    }

    fn origin(&self, column: usize) -> Option<FaceOrigin> {
        let span = self.rings.first()?.spans.get(column)?;
        Some(FaceOrigin::Side {
            feature: self.feature,
            entity: span.piece.entity(),
        })
    }

    fn build(&self, skin: LoftSkin) -> Result<Solid, LoftError> {
        let legs = self.legs(skin)?;
        let count = self.count;
        let rows = self.rows();
        let mut plan = Plan::default();
        let mut vertices: Vec<Vec<usize>> = Vec::with_capacity(rows);
        for ring in &self.rings {
            let row = (0..count)
                .map(|index| ring.vertex(index).map(|point| plan.vertex(point)))
                .collect::<Option<Vec<_>>>()
                .ok_or_else(degenerate)?;
            vertices.push(row);
        }
        if let Some(apex) = self.apex {
            vertices.push(vec![plan.vertex(apex); count]);
        }
        let vertex = |row: usize, index: usize| -> Result<usize, LoftError> {
            vertices
                .get(row)
                .and_then(|line| line.get(index % count.max(1)))
                .copied()
                .ok_or_else(degenerate)
        };
        let leg_of_row_above = |row: usize| legs.iter().position(|leg| leg.first == row);
        let leg_of_row_below = |row: usize| legs.iter().position(|leg| leg.last == row);
        let mut sections: Vec<Option<Vec<usize>>> = Vec::with_capacity(rows);
        for (row, ring) in self.rings.iter().enumerate() {
            let above = leg_of_row_above(row);
            let below = leg_of_row_below(row);
            if above.is_none() && below.is_none() {
                sections.push(None);
                continue;
            }
            let mut line = Vec::with_capacity(count);
            for index in 0..count {
                let name = match (below, above) {
                    (None, Some(leg)) => EdgeName::between(self.side(leg, index)?, self.start_cap),
                    (Some(leg), None) => EdgeName::between(self.side(leg, index)?, self.end_cap),
                    (Some(lower), Some(upper)) => {
                        EdgeName::between(self.side(lower, index)?, self.side(upper, index)?)
                    }
                    (None, None) => return Err(degenerate()),
                };
                let (curve, range) = ring.span_edge(index)?;
                let (from, to) = (vertex(row, index)?, vertex(row, index + 1)?);
                let reversed = ring.spans.get(index).is_some_and(|span| span.reversed);
                let ends = if reversed { (to, from) } else { (from, to) };
                line.push(plan.edge(curve, range, ends, name));
            }
            sections.push(Some(line));
        }
        let section_edge = |row: usize, index: usize| -> Result<(usize, Sense), LoftError> {
            let edge = sections
                .get(row)
                .and_then(Option::as_ref)
                .and_then(|line| line.get(index))
                .copied()
                .ok_or_else(degenerate)?;
            let reversed = self
                .rings
                .get(row)
                .and_then(|ring| ring.spans.get(index))
                .is_some_and(|span| span.reversed);
            Ok((
                edge,
                if reversed {
                    Sense::Reversed
                } else {
                    Sense::Same
                },
            ))
        };
        for (number, leg) in legs.iter().enumerate() {
            interrupt::check()?;
            let mut alongs = Vec::with_capacity(count);
            for index in 0..count {
                let name = if count == 1 {
                    EdgeName::seam(self.side(number, 0)?)
                } else {
                    EdgeName::between(
                        self.side(number, index + count - 1)?,
                        self.side(number, index)?,
                    )
                };
                let ends = (vertex(leg.first, index)?, vertex(leg.last, index)?);
                let edge = if leg.last == leg.first + 1 {
                    plan.line(ends.0, ends.1, name)?
                } else {
                    let surface = leg.surfaces.get(index).ok_or_else(degenerate)?;
                    let side = surface.side(SurfaceSide::UStart).ok_or_else(degenerate)?;
                    let interval = side.domain();
                    plan.edge(Curve::BSpline(side), interval, ends, name)
                };
                alongs.push(edge);
            }
            let along = |index: usize| {
                alongs
                    .get(index % count.max(1))
                    .copied()
                    .ok_or_else(degenerate)
            };
            let ring = self.rings.get(leg.first).ok_or_else(degenerate)?;
            for index in 0..count {
                let spline = leg.surfaces.get(index).ok_or_else(degenerate)?;
                let surface = Surface::from(spline.clone());
                let (probe, tangent) = ring.traversal_tangent(index).ok_or_else(degenerate)?;
                let outward = tangent.cross(ring.oriented_normal());
                let sense = outward_sense(&surface, probe, outward, None);
                let (bottom, travel) = section_edge(leg.first, index)?;
                let mut coedges = vec![
                    PlanCoedge::new(bottom, travel),
                    PlanCoedge::new(along(index + 1)?, Sense::Same),
                ];
                if leg.last < self.rings.len() {
                    let (top, top_travel) = section_edge(leg.last, index)?;
                    coedges.push(PlanCoedge::new(top, top_travel.reversed()));
                }
                coedges.push(PlanCoedge::new(along(index)?, Sense::Reversed));
                plan.face(PlanFace {
                    surface,
                    sense,
                    name: self.side(number, index)?,
                    origin: self.origin(index),
                    loops: vec![coedges],
                });
            }
        }
        self.caps(&mut plan, &section_edge)?;
        Ok(plan.build().map_err(SweepError::from)?)
    }

    fn caps(
        &self,
        plan: &mut Plan,
        section_edge: &dyn Fn(usize, usize) -> Result<(usize, Sense), LoftError>,
    ) -> Result<(), LoftError> {
        let first = self.rings.first().ok_or_else(degenerate)?;
        let start_plane = Plane::from_frame(
            first.sketch.origin(),
            -first.oriented_normal(),
            first.sketch.x_axis(),
        )
        .ok_or_else(degenerate)?;
        let start_loop = (0..self.count)
            .rev()
            .map(|index| {
                section_edge(0, index).map(|(edge, sense)| PlanCoedge::new(edge, sense.reversed()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        plan.face(PlanFace {
            surface: PlaneSurface::new(start_plane)?.into(),
            sense: Sense::Same,
            name: self.start_cap,
            origin: Some(FaceOrigin::StartCap {
                feature: self.feature,
            }),
            loops: vec![start_loop],
        });
        if self.apex.is_some() {
            return Ok(());
        }
        let row = self.rings.len() - 1;
        let last = self.rings.last().ok_or_else(degenerate)?;
        let end_plane = Plane::from_frame(
            last.sketch.origin(),
            last.oriented_normal(),
            last.sketch.x_axis(),
        )
        .ok_or_else(degenerate)?;
        let end_loop = (0..self.count)
            .map(|index| section_edge(row, index).map(|(edge, sense)| PlanCoedge::new(edge, sense)))
            .collect::<Result<Vec<_>, _>>()?;
        plan.face(PlanFace {
            surface: PlaneSurface::new(end_plane)?.into(),
            sense: Sense::Same,
            name: self.end_cap,
            origin: Some(FaceOrigin::EndCap {
                feature: self.feature,
            }),
            loops: vec![end_loop],
        });
        Ok(())
    }
}
