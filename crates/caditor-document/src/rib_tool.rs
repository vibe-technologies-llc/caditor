use std::collections::BTreeMap;

use caditor_geometry::{Aabb2, Plane, Point2, Point3, Vector2, Vector3};
use caditor_kernel::{
    BooleanError, BooleanOperation, FaceName, FaceOrigin, Interval, LINEAR_RESOLUTION,
    LinearExtent, PieceBound, PieceId, PointClass, Profile, ProfileCurve, ProfileError,
    ProfileShape, RayCrossing, Region, Solid, SweepError, TessellationError, WallError, WallSide,
    boolean, extrude, wall_regions,
};

use crate::split::{End, Perimeter, SweptError, chain, component_of, curve_links, heights};

pub const START_RUN: u64 = 1 << 62;
pub const END_RUN: u64 = 1 << 61;
const RUNS: u64 = START_RUN | END_RUN;
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

pub fn run_curve(entity: u64) -> u64 {
    entity & !RUNS
}

pub fn is_run(entity: u64) -> bool {
    entity & RUNS != 0 && !PERIMETER_ENTITIES.contains(&entity)
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
    pub side: RibSide,
    pub direction: RibDirection,
    pub reversed: bool,
    pub feature: u64,
}

struct Frame {
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
        reach: bounds.size().length() + heights.margin,
        bounds,
        low: heights.low,
        high: heights.high,
        margin: heights.margin,
    };
    let mut tools = Vec::new();
    for chain in &chains {
        tools.extend(chain_tools(input, &frame, chain)?);
    }
    Ok(tools)
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
    let mut parents: Vec<usize> = (0..2 * curves.len()).collect();
    for [first, second] in links.iter().flatten() {
        let root = component_of(&mut parents, *first);
        let other = component_of(&mut parents, *second);
        if let Some(parent) = parents.get_mut(other) {
            *parent = root;
        }
    }
    let mut groups: BTreeMap<usize, Vec<ProfileCurve>> = BTreeMap::new();
    for (curve, link) in curves.iter().zip(&links) {
        if let Some([first, _]) = link {
            let root = component_of(&mut parents, *first);
            groups.entry(root).or_default().push(curve.clone());
        }
    }
    groups
        .into_values()
        .map(|group| {
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
        })
        .collect()
}

fn walk_left(end: &End) -> Vector2 {
    let ahead = -end.outward;
    Vector2::new(-ahead.y, ahead.x)
}

fn in_plane(plane: &Plane, direction: Vector2) -> Vector3 {
    plane.x_axis() * direction.x + plane.y_axis() * direction.y
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
    Ends { names: Vec<(FaceName, u64)> },
    Depth { far: FaceOrigin },
    Perimeter,
}

fn chain_tools(
    input: &RibInput<'_>,
    frame: &Frame,
    chain: &RibChain,
) -> Result<Vec<Solid>, RibError> {
    let (slab, seeds, escapes) = match input.direction {
        RibDirection::Square => web_slab(input, frame, chain)?,
        RibDirection::Parallel => rib_slab(input, frame, chain)?,
    };
    let entities = chain.entities();
    let classifier = input.body.classifier();
    let seeds: Vec<Point3> = seeds
        .into_iter()
        .filter(|seed| classifier.classify(*seed) == PointClass::Outside)
        .collect();
    if seeds.is_empty() {
        return Err(RibError::Inside { entities });
    }
    let pieces = match boolean(&slab, input.body, BooleanOperation::Difference) {
        Ok(pieces) => pieces,
        Err(BooleanError::Empty) => return Err(RibError::Inside { entities }),
        Err(error) => return Err(RibError::Boolean(error)),
    };
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

fn web_slab(input: &RibInput<'_>, frame: &Frame, chain: &RibChain) -> Result<Slab, RibError> {
    let thickness = input.thickness;
    let (extended, far_ends) = carried_on(
        chain,
        chain.start.point + chain.start.outward * frame.reach,
        chain.finish.point + chain.finish.outward * frame.reach,
    );
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
    let slab = extrude(
        &input.plane,
        &regions,
        extent.map_err(RibError::Sweep)?,
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

fn rib_slab(input: &RibInput<'_>, frame: &Frame, chain: &RibChain) -> Result<Slab, RibError> {
    let thickness = input.thickness;
    let bounds = &frame.bounds;
    let perimeter = Perimeter::new(bounds);
    let entities = chain.entities();
    let degenerate = || RibError::Degenerate {
        entities: entities.clone(),
    };
    let start = perimeter
        .exit(bounds, &chain.start)
        .ok_or_else(degenerate)?;
    let finish = perimeter
        .exit(bounds, &chain.finish)
        .ok_or_else(degenerate)?;
    let corners = perimeter.walk(finish, start, !input.reversed);
    let (mut curves, _) = carried_on(chain, start.1, finish.1);
    let around = corners
        .iter()
        .copied()
        .zip(corners.iter().copied().skip(1))
        .filter(|(from, to)| from.distance(*to) > LINEAR_RESOLUTION);
    for (entity, (from, to)) in PERIMETER_ENTITIES.iter().zip(around) {
        curves.push(ProfileCurve::line(*entity, from, to));
    }
    let crosses = || RibError::CrossesItself {
        entities: entities.clone(),
    };
    let profile = Profile::new(&curves).map_err(|error| match error {
        ProfileError::NoClosedProfile { .. } => crosses(),
        other => RibError::Profile(other),
    })?;
    let regions = profile.regions();
    if regions.len() != 1 {
        return Err(crosses());
    }
    let (extent, middle) = match input.side {
        RibSide::OneSide => (LinearExtent::new(0.0, thickness), 0.5 * thickness),
        RibSide::OtherSide => (LinearExtent::new(-thickness, 0.0), -0.5 * thickness),
        RibSide::Symmetric => (LinearExtent::symmetric(thickness), 0.0),
    };
    let slab = extrude(
        &input.plane,
        regions,
        extent.map_err(RibError::Sweep)?,
        input.feature,
    )
    .map_err(RibError::Sweep)?;
    let depth = seed_depth(thickness);
    let normal = input.plane.normal();
    let seeds = seeds_in(regions, &chain.curves, &[depth, -depth])
        .into_iter()
        .map(|point| input.plane.to_world(point) + normal * middle)
        .collect();
    Ok((slab, seeds, vec![Escape::Perimeter]))
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
