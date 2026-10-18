use std::{
    collections::{BTreeMap, BTreeSet},
    f64::consts::TAU,
};

use caditor_geometry::{Aabb2, Plane, Point2, Point3, Vector2, Vector3};
use caditor_kernel::{
    BooleanError, BooleanOperation, FaceName, FaceOrigin, Interval, LINEAR_RESOLUTION,
    LinearExtent, PieceBound, PieceId, PointClass, Profile, ProfileCurve, ProfileError,
    ProfileShape, RayCrossing, Region, Solid, SweepError, TessellationError, WallError, WallSide,
    boolean, extrude, extrude_flared, extrude_tapered, wall_regions,
};

use crate::split::{End, Perimeter, SweptError, chain, curve_links, heights};

pub const START_RUN: u64 = 1 << 62;
pub const END_RUN: u64 = 1 << 61;
pub const FRONT_FLANK: u64 = 1 << 60;
pub const BACK_FLANK: u64 = 1 << 59;
const RUNS: u64 = START_RUN | END_RUN;
const FLANKS: u64 = FRONT_FLANK | BACK_FLANK;
const PERIMETER_ENTITIES: [u64; 5] = [
    u64::MAX - 20,
    u64::MAX - 21,
    u64::MAX - 22,
    u64::MAX - 23,
    u64::MAX - 24,
];
const SEED_FRACTIONS: [f64; 3] = [0.25, 0.5, 0.75];
const SEED_REACH: f64 = 0.05;
const SEED_SHARE: f64 = 0.25;
const ARC_GAP_DEGREES: f64 = 2.0;
const RIM_REACH: f64 = 4.0;
const RIM_SLACK: f64 = 4.0;
const FLANK_SAMPLES: usize = 64;
const FLANK_DEPTH_SCALES: [f64; 4] = [1.0, 0.5, 0.25, 0.125];
const FLANK_TOLERANCE: f64 = 1e-5;
const FLANK_SAFETY: f64 = 1.1;
const FLANK_SLACK: f64 = 1.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum RibSide {
    OneSide,
    OtherSide,
    #[default]
    Symmetric,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum RibDirection {
    #[default]
    Parallel,
    Square,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum RibEnds {
    #[default]
    Straight,
    RoundArcs,
}

pub fn run_curve(entity: u64) -> u64 {
    if PERIMETER_ENTITIES.contains(&entity) {
        entity
    } else {
        entity & !(RUNS | FLANKS)
    }
}

pub fn is_run(entity: u64) -> bool {
    entity & RUNS != 0 && !PERIMETER_ENTITIES.contains(&entity)
}

pub fn is_flank(entity: u64) -> bool {
    entity & FLANKS != 0 && !PERIMETER_ENTITIES.contains(&entity)
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum RibError {
    #[error("the sketch has no curve")]
    NoCurves,
    #[error("the curves close on themselves")]
    Closed { entities: Vec<u64> },
    #[error("three or more curves meet at one point")]
    Branches { entities: Vec<u64> },
    #[error("a curve has no direction at its end")]
    Degenerate { entities: Vec<u64> },
    #[error("carried on past its ends, the chain crosses itself")]
    CrossesItself { entities: Vec<u64> },
    #[error("the body has no extent")]
    NoExtent,
    #[error("the body lies wholly behind the sketch")]
    Behind,
    #[error("the rib lies inside the body")]
    Inside { entities: Vec<u64> },
    #[error("the rib meets no face of the body")]
    MeetsNothing { entities: Vec<u64> },
    #[error("an end of the rib runs on past the body")]
    RunsPastEnd { entity: u64 },
    #[error("the rib runs on past the body beyond its sketch")]
    RunsPastDepth { entities: Vec<u64> },
    #[error("the rib runs on past the end of its drafted faces")]
    RunsPastFlank { entities: Vec<u64> },
    #[error("the rib runs on past the body")]
    RunsPast { entities: Vec<u64> },
    #[error("the wall could not be drawn: {0}")]
    Wall(WallError),
    #[error("the outline could not be drawn: {0}")]
    Profile(ProfileError),
    #[error("it could not be extruded: {0}")]
    Sweep(SweepError),
    #[error(transparent)]
    Boolean(BooleanError),
    #[error("its pieces could not be told apart: {0}")]
    Lumps(TessellationError),
}

pub(crate) struct RibInput<'a> {
    pub plane: Plane,
    pub curves: Vec<ProfileCurve>,
    pub body: &'a Solid,
    pub thickness: f64,
    pub draft: f64,
    pub side: RibSide,
    pub direction: RibDirection,
    pub ends: RibEnds,
    pub reversed: bool,
    pub feature: u64,
}

struct Frame {
    footprint: Vec<Point2>,
    circle: (Point2, f64),
    bounds: Aabb2,
    reach: f64,
    low: f64,
    high: f64,
    margin: f64,
}

pub(crate) fn rib_tools(input: &RibInput<'_>) -> Result<Vec<Solid>, RibError> {
    let chains = chains(&input.curves)?;
    let heights =
        heights(input.body, &input.plane, input.plane.normal()).ok_or(RibError::NoExtent)?;
    let bounds = chains.iter().fold(heights.across, |bounds, chain| {
        bounds.union(chain.bounds.expanded(heights.margin))
    });
    let frame = Frame {
        footprint: footprint(input.body, &input.plane),
        circle: (
            bounds.center(),
            RIM_REACH * bounds.size().length() + RIM_SLACK,
        ),
        reach: bounds.size().length() + heights.margin,
        bounds,
        low: heights.low,
        high: heights.high,
        margin: heights.margin,
    };
    let attached = attachments(&chains);
    let mut built: Vec<Option<Vec<Solid>>> = vec![None; chains.len()];
    for index in build_order(&attached) {
        let Some(chain) = chains.get(index) else {
            continue;
        };
        let obstacles: Vec<&Solid> = attached
            .get(index)
            .into_iter()
            .flatten()
            .filter_map(|other| built.get(*other))
            .flatten()
            .flatten()
            .collect();
        let tools = chain_tools(input, &frame, chain, &obstacles)?;
        if let Some(slot) = built.get_mut(index) {
            *slot = Some(tools);
        }
    }
    Ok(built.into_iter().flatten().flatten().collect())
}

fn footprint(body: &Solid, plane: &Plane) -> Vec<Point2> {
    body.bounding_box()
        .map(|bounds| {
            bounds
                .corners()
                .iter()
                .map(|corner| {
                    let offset = *corner - plane.origin();
                    Point2::new(offset.dot(plane.x_axis()), offset.dot(plane.y_axis()))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn flank_depth(frame: &Frame, chain: &RibChain) -> f64 {
    let along: Vec<Point2> = chain
        .curves
        .iter()
        .filter_map(|curve| curve.curve().ok())
        .flat_map(|(path, range): (_, Interval)| {
            (0..=FLANK_SAMPLES)
                .map(|step| path.point(range.at(step as f64 / FLANK_SAMPLES as f64)))
                .collect::<Vec<_>>()
        })
        .collect();
    let reach = frame
        .footprint
        .iter()
        .map(|corner| {
            along
                .iter()
                .map(|point| point.distance(*corner))
                .fold(f64::INFINITY, f64::min)
        })
        .filter(|distance| distance.is_finite())
        .fold(0.0, f64::max);
    reach * FLANK_SAFETY + FLANK_SLACK
}

pub(crate) struct RibChain {
    curves: Vec<ProfileCurve>,
    start: End,
    finish: End,
    bounds: Aabb2,
}

impl RibChain {
    fn entities(&self) -> Vec<u64> {
        let mut entities: Vec<u64> = self.curves.iter().map(|curve| curve.entity).collect();
        entities.sort_unstable();
        entities
    }

    fn passes_by(&self, point: Point2) -> bool {
        self.curves.iter().any(|curve| {
            curve.curve().is_ok_and(|(path, range)| {
                let along = path.closest_parameter(point, range);
                path.point(along).distance(point) <= LINEAR_RESOLUTION
            })
        })
    }
}

pub(crate) fn chains(curves: &[ProfileCurve]) -> Result<Vec<RibChain>, RibError> {
    if curves.is_empty() {
        return Err(RibError::NoCurves);
    }
    let links = curve_links(curves);
    let closed: Vec<u64> = curves
        .iter()
        .zip(&links)
        .filter(|(_, link)| link.is_none())
        .map(|(curve, _)| curve.entity)
        .collect();
    if !closed.is_empty() {
        return Err(RibError::Closed { entities: closed });
    }
    let outward = end_directions(curves)?;
    let partners = paired(&links, &outward);
    let mut taken = vec![false; curves.len()];
    let mut walks = Vec::new();
    for (slot, partner) in partners.iter().enumerate() {
        if partner.is_none() && taken.get(slot / 2) == Some(&false) {
            walks.push(walked(&partners, slot, &mut taken));
        }
    }
    let looped: Vec<u64> = curves
        .iter()
        .zip(&taken)
        .filter(|(_, taken)| !**taken)
        .map(|(curve, _)| curve.entity)
        .collect();
    if !looped.is_empty() {
        return Err(RibError::Closed { entities: looped });
    }
    walks
        .into_iter()
        .map(|walk| {
            let group: Vec<ProfileCurve> = walk
                .iter()
                .filter_map(|index| curves.get(*index).cloned())
                .collect();
            rib_chain(group)
        })
        .collect()
}

fn end_directions(curves: &[ProfileCurve]) -> Result<Vec<Vector2>, RibError> {
    let mut directions = Vec::with_capacity(2 * curves.len());
    for curve in curves {
        let degenerate = || RibError::Degenerate {
            entities: vec![curve.entity],
        };
        let (path, range) = curve.curve().map_err(RibError::Profile)?;
        let first = path.evaluate(range.start()).first.try_normalize();
        let last = path.evaluate(range.end()).first.try_normalize();
        directions.push(-first.ok_or_else(degenerate)?);
        directions.push(last.ok_or_else(degenerate)?);
    }
    Ok(directions)
}

fn paired(links: &[Option<[usize; 2]>], outward: &[Vector2]) -> Vec<Option<usize>> {
    let mut at_nodes: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (index, link) in links.iter().enumerate() {
        if let Some([first, last]) = link {
            at_nodes.entry(*first).or_default().push(2 * index);
            at_nodes.entry(*last).or_default().push(2 * index + 1);
        }
    }
    let mut partners = vec![None; 2 * links.len()];
    for mut open in at_nodes.into_values() {
        while let Some((one, other)) = straightest(&open, outward) {
            for (slot, partner) in [(one, other), (other, one)] {
                if let Some(entry) = partners.get_mut(slot) {
                    *entry = Some(partner);
                }
            }
            open.retain(|slot| *slot != one && *slot != other);
        }
    }
    partners
}

fn straightest(open: &[usize], outward: &[Vector2]) -> Option<(usize, usize)> {
    let turn = |slot: usize, other: usize| {
        outward
            .get(slot)
            .zip(outward.get(other))
            .map_or(f64::INFINITY, |(one, other)| one.dot(*other))
    };
    open.iter()
        .enumerate()
        .flat_map(|(at, one)| open.iter().skip(at + 1).map(move |other| (*one, *other)))
        .min_by(|(one, other), (third, fourth)| {
            turn(*one, *other).total_cmp(&turn(*third, *fourth))
        })
}

fn walked(partners: &[Option<usize>], from: usize, taken: &mut [bool]) -> Vec<usize> {
    let mut order = Vec::new();
    let mut at = from;
    while let Some(slot) = taken.get_mut(at / 2).filter(|taken| !**taken) {
        *slot = true;
        order.push(at / 2);
        match partners.get(at ^ 1).copied().flatten() {
            Some(next) => at = next,
            None => break,
        }
    }
    order
}

fn rib_chain(group: Vec<ProfileCurve>) -> Result<RibChain, RibError> {
    let entities: Vec<u64> = group.iter().map(|curve| curve.entity).collect();
    let found = chain(&group).map_err(|error| match error {
        SweptError::Closed => RibError::Closed {
            entities: entities.clone(),
        },
        SweptError::NotOneChain => RibError::Branches {
            entities: entities.clone(),
        },
        SweptError::Profile(error) => RibError::Profile(error),
        _ => RibError::Degenerate {
            entities: entities.clone(),
        },
    })?;
    Ok(RibChain {
        curves: group,
        start: found.start,
        finish: found.finish,
        bounds: found.bounds,
    })
}

fn attachments(chains: &[RibChain]) -> Vec<BTreeSet<usize>> {
    chains
        .iter()
        .enumerate()
        .map(|(index, chain)| {
            chains
                .iter()
                .enumerate()
                .filter(|(other, _)| *other != index)
                .filter(|(_, other)| {
                    [chain.start.point, chain.finish.point]
                        .into_iter()
                        .any(|end| other.passes_by(end))
                })
                .map(|(other, _)| other)
                .collect()
        })
        .collect()
}

fn build_order(attached: &[BTreeSet<usize>]) -> Vec<usize> {
    let mut done = vec![false; attached.len()];
    let mut order = Vec::with_capacity(attached.len());
    let waiting = |done: &[bool], index: usize| done.get(index) == Some(&false);
    while order.len() < attached.len() {
        let ready = (0..attached.len()).find(|index| {
            waiting(&done, *index)
                && attached
                    .get(*index)
                    .is_none_or(|others| others.iter().all(|other| !waiting(&done, *other)))
        });
        let Some(next) = ready.or_else(|| (0..attached.len()).find(|index| waiting(&done, *index)))
        else {
            break;
        };
        if let Some(slot) = done.get_mut(next) {
            *slot = true;
        }
        order.push(next);
    }
    order
}

fn walk_left(end: &End) -> Vector2 {
    let ahead = -end.outward;
    Vector2::new(-ahead.y, ahead.x)
}

fn in_plane(plane: &Plane, direction: Vector2) -> Vector3 {
    plane.x_axis() * direction.x + plane.y_axis() * direction.y
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThicknessStand {
    pub base: Point3,
    pub direction: Vector3,
    pub share: f64,
}

pub(crate) fn thickness_stand(
    plane: &Plane,
    curves: &[ProfileCurve],
    side: RibSide,
    direction: RibDirection,
) -> Option<ThicknessStand> {
    let chains = chains(curves).ok()?;
    let chain = chains.first()?;
    let (base, toward) = match direction {
        RibDirection::Parallel => {
            let sample = samples(&chain.curves).into_iter().next()?;
            (plane.to_world(sample.point), plane.normal())
        }
        RibDirection::Square => (
            plane.to_world(chain.start.point),
            in_plane(plane, walk_left(&chain.start)),
        ),
    };
    let (direction, share) = match side {
        RibSide::OneSide => (toward, 1.0),
        RibSide::OtherSide => (-toward, 1.0),
        RibSide::Symmetric => (toward, 0.5),
    };
    Some(ThicknessStand {
        base,
        direction,
        share,
    })
}

struct Sample {
    point: Point2,
    left: Vector2,
}

fn samples(curves: &[ProfileCurve]) -> Vec<Sample> {
    curves
        .iter()
        .filter_map(|curve| curve.curve().ok())
        .flat_map(|(path, range): (_, Interval)| {
            SEED_FRACTIONS
                .iter()
                .filter_map(|fraction| {
                    let at = path.evaluate(range.start() + range.length() * fraction);
                    let ahead = at.first.try_normalize()?;
                    Some(Sample {
                        point: at.point,
                        left: Vector2::new(-ahead.y, ahead.x),
                    })
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn seed_depth(thickness: f64) -> f64 {
    (thickness * SEED_SHARE).clamp(100.0 * LINEAR_RESOLUTION, SEED_REACH)
}

fn seeds_in(regions: &[Region], curves: &[ProfileCurve], reaches: &[f64]) -> Vec<Point2> {
    samples(curves)
        .into_iter()
        .filter_map(|sample| {
            reaches
                .iter()
                .map(|reach| sample.point + sample.left * *reach)
                .find(|point| regions.iter().any(|region| region.contains(*point)))
        })
        .collect()
}

enum Escape {
    Flank { low: f64, high: f64 },
    Ends { names: Vec<(FaceName, u64)> },
    Depth { far: FaceOrigin },
    Perimeter,
}

fn chain_tools(
    input: &RibInput<'_>,
    frame: &Frame,
    chain: &RibChain,
    obstacles: &[&Solid],
) -> Result<Vec<Solid>, RibError> {
    let (slab, seeds, escapes) = match input.direction {
        RibDirection::Square => web_slab(input, frame, chain)?,
        RibDirection::Parallel => flanked_slab(input, frame, chain)?,
    };
    let entities = chain.entities();
    let blocking: Vec<&Solid> = std::iter::once(input.body)
        .chain(obstacles.iter().copied())
        .collect();
    let classifiers: Vec<_> = blocking.iter().map(|solid| solid.classifier()).collect();
    let seeds: Vec<Point3> = seeds
        .into_iter()
        .filter(|seed| {
            classifiers
                .iter()
                .all(|classifier| classifier.classify(*seed) == PointClass::Outside)
        })
        .collect();
    if seeds.is_empty() {
        return Err(RibError::Inside { entities });
    }
    let mut pieces = slab;
    for solid in blocking {
        pieces = match boolean(&pieces, solid, BooleanOperation::Difference) {
            Ok(pieces) => pieces,
            Err(BooleanError::Empty) => return Err(RibError::Inside { entities }),
            Err(error) => return Err(RibError::Boolean(error)),
        };
    }
    let kept: Vec<Solid> = pieces
        .lumps()
        .map_err(RibError::Lumps)?
        .into_iter()
        .filter(|lump| {
            let classifier = lump.classifier();
            seeds
                .iter()
                .any(|seed| classifier.classify(*seed) == PointClass::Inside)
        })
        .collect();
    if kept.is_empty() {
        return Err(RibError::Inside { entities });
    }
    for escape in &escapes {
        if let Escape::Flank { low, high } = escape {
            let outside = kept.iter().any(|lump| {
                lump.bounding_box().is_some_and(|bounds| {
                    bounds.corners().iter().any(|corner| {
                        let height = (*corner - input.plane.origin()).dot(input.plane.normal());
                        height >= *high - FLANK_TOLERANCE || height <= *low + FLANK_TOLERANCE
                    })
                })
            });
            if outside {
                return Err(RibError::RunsPastFlank { entities });
            }
        }
    }
    for lump in &kept {
        let origins: Vec<(FaceName, Option<FaceOrigin>)> = lump
            .faces()
            .map(|(_, face)| (face.name(), face.origin()))
            .collect();
        if origins
            .iter()
            .all(|(_, origin)| origin.is_some_and(|origin| origin.feature() == input.feature))
        {
            return Err(RibError::MeetsNothing { entities });
        }
        for escape in &escapes {
            escaped(escape, &origins, &entities)?;
        }
    }
    Ok(kept)
}

fn escaped(
    escape: &Escape,
    origins: &[(FaceName, Option<FaceOrigin>)],
    entities: &[u64],
) -> Result<(), RibError> {
    match escape {
        Escape::Ends { names } => {
            if let Some((_, entity)) = names
                .iter()
                .find(|(name, _)| origins.iter().any(|(face, _)| face == name))
            {
                return Err(RibError::RunsPastEnd { entity: *entity });
            }
            Ok(())
        }
        Escape::Depth { far } => {
            if origins.iter().any(|(_, origin)| *origin == Some(*far)) {
                return Err(RibError::RunsPastDepth {
                    entities: entities.to_vec(),
                });
            }
            Ok(())
        }
        Escape::Flank { .. } => Ok(()),
        Escape::Perimeter => {
            let past = origins.iter().any(|(_, origin)| {
                matches!(origin, Some(FaceOrigin::Side { entity, .. }) if PERIMETER_ENTITIES.contains(entity))
            });
            if past {
                return Err(RibError::RunsPast {
                    entities: entities.to_vec(),
                });
            }
            Ok(())
        }
    }
}

type Slab = (Solid, Vec<Point3>, Vec<Escape>);

fn carried_to(curves: &mut Vec<ProfileCurve>, end: &End, run: u64, to: Point2) -> u64 {
    let (entity, at_end) = end.order;
    if end.point.distance(to) <= LINEAR_RESOLUTION {
        return entity;
    }
    let lengthened = curves.iter_mut().find_map(|curve| match &mut curve.shape {
        ProfileShape::Line { start, end } if curve.entity == entity => {
            Some(if at_end { end } else { start })
        }
        _ => None,
    });
    if let Some(point) = lengthened {
        *point = to;
        return entity;
    }
    let carried = run | entity;
    curves.push(if at_end {
        ProfileCurve::line(carried, end.point, to)
    } else {
        ProfileCurve::line(carried, to, end.point)
    });
    carried
}

fn carried_on(chain: &RibChain, start: Point2, finish: Point2) -> (Vec<ProfileCurve>, [u64; 2]) {
    let mut curves = chain.curves.clone();
    let first = carried_to(&mut curves, &chain.start, START_RUN, start);
    let last = carried_to(&mut curves, &chain.finish, END_RUN, finish);
    (curves, [first, last])
}

fn bent_round(curves: &mut [ProfileCurve], drawn: &[ProfileCurve], end: &End) -> Option<u64> {
    let (entity, at_end) = end.order;
    let ProfileShape::Arc {
        center,
        start,
        end: finish,
    } = drawn.iter().find(|curve| curve.entity == entity)?.shape
    else {
        return None;
    };
    let angle = |point: Point2| (point.y - center.y).atan2(point.x - center.x);
    let sweep = (angle(finish) - angle(start)).rem_euclid(TAU);
    let reach = (0.5 * (TAU - sweep - ARC_GAP_DEGREES.to_radians())).max(0.0);
    let turned = if at_end {
        angle(finish) + reach
    } else {
        angle(start) - reach
    };
    let point = center + Vector2::new(turned.cos(), turned.sin()) * start.distance(center);
    let moved = curves.iter_mut().find_map(|curve| match &mut curve.shape {
        ProfileShape::Arc { start, end, .. } if curve.entity == entity => {
            Some(if at_end { end } else { start })
        }
        _ => None,
    })?;
    *moved = point;
    Some(entity)
}

fn carried_by(chain: &RibChain, frame: &Frame, ends: RibEnds) -> (Vec<ProfileCurve>, [u64; 2]) {
    let mut curves = chain.curves.clone();
    let mut carry = |end: &End, run: u64| {
        let round = match ends {
            RibEnds::RoundArcs => bent_round(&mut curves, &chain.curves, end),
            RibEnds::Straight => None,
        };
        round.unwrap_or_else(|| {
            carried_to(&mut curves, end, run, end.point + end.outward * frame.reach)
        })
    };
    let first = carry(&chain.start, START_RUN);
    let last = carry(&chain.finish, END_RUN);
    (curves, [first, last])
}

fn web_slab(input: &RibInput<'_>, frame: &Frame, chain: &RibChain) -> Result<Slab, RibError> {
    let thickness = input.thickness;
    let (extended, far_ends) = carried_by(chain, frame, input.ends);
    let regions = match input.side {
        RibSide::Symmetric => {
            wall_regions(&extended, thickness, WallSide::Centred).map_err(RibError::Wall)?
        }
        RibSide::OneSide | RibSide::OtherSide => {
            let left = walk_left(&chain.start);
            let aim = if input.side == RibSide::OneSide {
                left
            } else {
                -left
            };
            let probe = chain.start.point + aim * (0.5 * thickness);
            let inside =
                wall_regions(&extended, thickness, WallSide::Inside).map_err(RibError::Wall)?;
            if inside.iter().any(|region| region.contains(probe)) {
                inside
            } else {
                wall_regions(&extended, thickness, WallSide::Outside).map_err(RibError::Wall)?
            }
        }
    };
    let feature = input.feature;
    let (extent, far, ahead) = if input.reversed {
        if frame.low >= -LINEAR_RESOLUTION {
            return Err(RibError::Behind);
        }
        (
            LinearExtent::new(frame.low - frame.margin, 0.0),
            FaceOrigin::StartCap { feature },
            -input.plane.normal(),
        )
    } else {
        if frame.high <= LINEAR_RESOLUTION {
            return Err(RibError::Behind);
        }
        (
            LinearExtent::new(0.0, frame.high + frame.margin),
            FaceOrigin::EndCap { feature },
            input.plane.normal(),
        )
    };
    let slab = extrude_tapered(
        &input.plane,
        &regions,
        extent.map_err(RibError::Sweep)?,
        -input.draft,
        feature,
    )
    .map_err(RibError::Sweep)?;
    let half = 0.5 * thickness;
    let reaches: Vec<f64> = match input.side {
        RibSide::Symmetric => vec![0.0, half, -half],
        RibSide::OneSide | RibSide::OtherSide => vec![half, -half],
    };
    let depth = seed_depth(thickness);
    let seeds = seeds_in(&regions, &chain.curves, &reaches)
        .into_iter()
        .map(|point| input.plane.to_world(point) + ahead * depth)
        .collect();
    let cap = |entity: u64, at_end: bool| {
        let bound = || {
            if at_end {
                PieceBound::End
            } else {
                PieceBound::Start
            }
        };
        FaceName::side(feature, &PieceId::new(entity, bound(), bound()))
    };
    let names = far_ends
        .into_iter()
        .flat_map(|run| {
            [
                (cap(run, false), run_curve(run)),
                (cap(run, true), run_curve(run)),
            ]
        })
        .collect();
    Ok((
        slab,
        seeds,
        vec![Escape::Ends { names }, Escape::Depth { far }],
    ))
}

fn rim_closed(
    curves: &[ProfileCurve],
    rim: &[ProfileCurve],
    mark: u64,
    crosses: impl Fn() -> RibError,
) -> Result<Vec<Region>, RibError> {
    let marked: Vec<ProfileCurve> = curves
        .iter()
        .map(|curve| ProfileCurve {
            entity: curve.entity | mark,
            ..curve.clone()
        })
        .chain(rim.iter().cloned())
        .collect();
    let profile = Profile::new(&marked).map_err(|error| match error {
        ProfileError::NoClosedProfile { .. } => crosses(),
        other => RibError::Profile(other),
    })?;
    let regions = profile.regions();
    if regions.len() != 1 {
        return Err(crosses());
    }
    Ok(regions.to_vec())
}

fn rectangular_rim(
    input: &RibInput<'_>,
    bounds: &Aabb2,
    chain: &RibChain,
) -> Option<(Point2, Point2, Vec<ProfileCurve>)> {
    let perimeter = Perimeter::new(bounds);
    let start = perimeter.exit(bounds, &chain.start)?;
    let finish = perimeter.exit(bounds, &chain.finish)?;
    let corners = perimeter.walk(finish, start, !input.reversed);
    let rim = PERIMETER_ENTITIES
        .iter()
        .zip(
            corners
                .iter()
                .copied()
                .zip(corners.iter().copied().skip(1))
                .filter(|(from, to)| from.distance(*to) > LINEAR_RESOLUTION),
        )
        .map(|(entity, (from, to))| ProfileCurve::line(*entity, from, to))
        .collect();
    Some((start.1, finish.1, rim))
}

fn circular_rim(
    input: &RibInput<'_>,
    frame: &Frame,
    chain: &RibChain,
) -> Option<(Point2, Point2, Vec<ProfileCurve>)> {
    let (centre, radius) = frame.circle;
    let exit = |end: &End| -> Option<Point2> {
        let gap = end.point - centre;
        let along = gap.dot(end.outward);
        let beyond = gap.length_squared() - radius * radius;
        let discriminant = along * along - beyond;
        (discriminant >= 0.0).then(|| end.point + end.outward * (discriminant.sqrt() - along))
    };
    let (start, finish) = (exit(&chain.start)?, exit(&chain.finish)?);
    let arc = if input.reversed {
        ProfileCurve::arc(PERIMETER_ENTITIES[0], centre, start, finish)
    } else {
        ProfileCurve::arc(PERIMETER_ENTITIES[0], centre, finish, start)
    };
    Some((start, finish, vec![arc]))
}

fn flanked_slab(input: &RibInput<'_>, frame: &Frame, chain: &RibChain) -> Result<Slab, RibError> {
    let scales: &[f64] = if input.draft > 0.0 {
        &FLANK_DEPTH_SCALES
    } else {
        &[1.0]
    };
    let mut attempts = scales.iter();
    loop {
        let Some(scale) = attempts.next() else {
            return rib_slab(input, frame, chain, 1.0);
        };
        match rib_slab(input, frame, chain, *scale) {
            Err(RibError::Sweep(SweepError::TaperCloses { .. })) if attempts.len() > 0 => {}
            other => return other,
        }
    }
}

fn rib_slab(
    input: &RibInput<'_>,
    frame: &Frame,
    chain: &RibChain,
    scale: f64,
) -> Result<Slab, RibError> {
    let thickness = input.thickness;
    let bounds = &frame.bounds;
    let entities = chain.entities();
    let degenerate = || RibError::Degenerate {
        entities: entities.clone(),
    };
    let (start, finish, rim) = if input.draft > 0.0 {
        circular_rim(input, frame, chain).ok_or_else(degenerate)?
    } else {
        rectangular_rim(input, bounds, chain).ok_or_else(degenerate)?
    };
    let (curves, _) = carried_on(chain, start, finish);
    let crosses = || RibError::CrossesItself {
        entities: entities.clone(),
    };
    let regions = rim_closed(&curves, &rim, 0, crosses)?;
    let (low, high, middle) = match input.side {
        RibSide::OneSide => (0.0, thickness, 0.5 * thickness),
        RibSide::OtherSide => (-thickness, 0.0, -0.5 * thickness),
        RibSide::Symmetric => (-0.5 * thickness, 0.5 * thickness, 0.0),
    };
    let extent = LinearExtent::new(low, high).map_err(RibError::Sweep)?;
    let mut slab =
        extrude(&input.plane, &regions, extent, input.feature).map_err(RibError::Sweep)?;
    if input.draft > 0.0 {
        let depth = flank_depth(frame, chain) * scale;
        let rise = depth * input.draft.tan();
        let flanks = [
            (input.side != RibSide::OtherSide).then_some((FRONT_FLANK, high, high + rise)),
            (input.side != RibSide::OneSide).then_some((BACK_FLANK, low, low - rise)),
        ];
        for (mark, from, to) in flanks.into_iter().flatten() {
            let flank_regions = rim_closed(&curves, &rim, mark, crosses)?;
            let flank = extrude_flared(
                &input.plane,
                &flank_regions,
                LinearExtent::new(from, to).map_err(RibError::Sweep)?,
                input.draft,
                input.feature,
            )
            .map_err(RibError::Sweep)?;
            slab = boolean(&slab, &flank, BooleanOperation::Union).map_err(RibError::Boolean)?;
        }
    }
    let seed_depth = seed_depth(thickness);
    let normal = input.plane.normal();
    let seeds = seeds_in(&regions, &chain.curves, &[seed_depth, -seed_depth])
        .into_iter()
        .map(|point| input.plane.to_world(point) + normal * middle)
        .collect();
    let mut escapes = vec![Escape::Perimeter];
    if input.draft > 0.0 {
        let rise = flank_depth(frame, chain) * scale * input.draft.tan();
        escapes.push(Escape::Flank {
            low: if input.side == RibSide::OneSide {
                f64::NEG_INFINITY
            } else {
                low - rise
            },
            high: if input.side == RibSide::OtherSide {
                f64::INFINITY
            } else {
                high + rise
            },
        });
    }
    Ok((slab, seeds, escapes))
}

pub(crate) fn facing_body(
    plane: &Plane,
    curves: &[ProfileCurve],
    body: &Solid,
    direction: RibDirection,
) -> Option<bool> {
    let chains = chains(curves).ok()?;
    let chain = chains.first()?;
    let middle = chain.start.point + (chain.finish.point - chain.start.point) * 0.5;
    let (from, ahead) = match direction {
        RibDirection::Square => {
            let sample = samples(&chain.curves).into_iter().next()?;
            (plane.to_world(sample.point), plane.normal())
        }
        RibDirection::Parallel => {
            let chord = (chain.finish.point - chain.start.point).try_normalize()?;
            let left = Vector2::new(-chord.y, chord.x);
            (plane.to_world(middle), in_plane(plane, left))
        }
    };
    let classifier = body.classifier();
    let reach = |toward: Vector3| match classifier.first_crossing(from, toward, 0.0) {
        RayCrossing::Crossing { distance, .. } => Some(distance),
        RayCrossing::Nothing | RayCrossing::Undecided => None,
    };
    match (reach(ahead), reach(-ahead)) {
        (Some(forward), Some(backward)) => Some(backward < forward),
        (None, Some(_)) => Some(true),
        (Some(_), None) => Some(false),
        (None, None) => None,
    }
}
