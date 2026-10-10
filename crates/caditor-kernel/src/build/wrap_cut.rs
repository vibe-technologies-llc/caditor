use std::{
    collections::{BTreeMap, VecDeque},
    f64::consts::TAU,
};

use caditor_geometry::{Plane, Point2, Vector2};

use crate::{
    build::{
        revolve::{arc_spline, ellipse_spline},
        wrap::{Wrap, WrapError, Wrapping, least_radius, wrapped},
    },
    curve2::Curve2,
    interrupt,
    profile::{PieceId, Profile, ProfileCurve, ProfileShape, Region},
    surface::Surface,
    tolerance::{LINEAR_RESOLUTION, SamplingTolerance},
    topology::{EdgeId, FaceId, Solid},
};

const MAX_TURNS: f64 = 64.0;
const MARGIN: f64 = 0.05;
const APEX_SHARE: f64 = 0.25;
const SEAM_TOLERANCE: f64 = 1e-6;
const OPEN_GAP: f64 = 1e-3;
const EDGE_OVERLAP: f64 = 10.0 * LINEAR_RESOLUTION;
const SAMPLING_ANGLE: f64 = 0.05;

#[derive(Debug, Clone, Copy)]
struct ChainEnd {
    point: Point2,
    outward: Vector2,
    curve: usize,
    at_end: bool,
    entity: u64,
}

fn chain_ends(curves: &[ProfileCurve]) -> Result<[ChainEnd; 2], WrapError> {
    let mut ends = Vec::with_capacity(2 * curves.len());
    for (index, curve) in curves.iter().enumerate() {
        if matches!(
            curve.shape,
            ProfileShape::Circle { .. } | ProfileShape::Ellipse { .. }
        ) {
            return Err(WrapError::NotOneChain);
        }
        let (path, range) = curve.curve().map_err(|_| WrapError::NotOneChain)?;
        let first = path.evaluate(range.start());
        let last = path.evaluate(range.end());
        if first.point.distance(last.point) <= LINEAR_RESOLUTION {
            return Err(WrapError::NotOneChain);
        }
        let direction = |tangent: Vector2| tangent.try_normalize().ok_or(WrapError::NotOneChain);
        ends.push(ChainEnd {
            point: first.point,
            outward: -direction(first.first)?,
            curve: index,
            at_end: false,
            entity: curve.entity,
        });
        ends.push(ChainEnd {
            point: last.point,
            outward: direction(last.first)?,
            curve: index,
            at_end: true,
            entity: curve.entity,
        });
    }
    let mut free = Vec::with_capacity(2);
    for end in &ends {
        let meeting = ends
            .iter()
            .filter(|other| other.point.distance(end.point) <= LINEAR_RESOLUTION)
            .count();
        match meeting {
            1 => free.push(*end),
            2 => {}
            _ => return Err(WrapError::NotOneChain),
        }
    }
    let [one, other] = free.as_slice() else {
        return Err(WrapError::NotOneChain);
    };
    Ok(
        if (one.entity, one.at_end) <= (other.entity, other.at_end) {
            [*one, *other]
        } else {
            [*other, *one]
        },
    )
}

#[derive(Debug, Clone, Copy)]
struct Span {
    start: f64,
    length: f64,
}

impl Span {
    fn holds(&self, angle: f64) -> bool {
        let ahead = (angle - self.start).rem_euclid(TAU);
        ahead <= self.length + SEAM_TOLERANCE || ahead >= TAU - SEAM_TOLERANCE
    }

    fn meet(&self, other: &Span) -> Option<Span> {
        let ahead = (other.start - self.start).rem_euclid(TAU);
        [ahead, ahead - TAU]
            .into_iter()
            .map(|from| {
                let low = from.max(0.0);
                let high = (from + other.length).min(self.length);
                (low, high)
            })
            .filter(|(low, high)| *high >= *low - SEAM_TOLERANCE)
            .max_by(|one, two| (one.1 - one.0).total_cmp(&(two.1 - two.0)))
            .map(|(low, high)| Span {
                start: self.start + low,
                length: (high - low).max(0.0),
            })
    }
}

struct Coverage {
    seam: Option<f64>,
    gap: Option<Span>,
    heights: (f64, f64),
}

fn coverage(
    wrap: &Wrap<'_>,
    solid: &Solid,
    face: FaceId,
    tolerance: &SamplingTolerance,
) -> Result<Coverage, WrapError> {
    let face = solid.face(face).ok_or(WrapError::Misses)?;
    let mut uses: BTreeMap<EdgeId, usize> = BTreeMap::new();
    for loop_id in face.loops() {
        let found = solid.face_loop(*loop_id).ok_or(WrapError::Unassembled)?;
        for coedge in found.coedges() {
            let coedge = solid.coedge(*coedge).ok_or(WrapError::Unassembled)?;
            *uses.entry(coedge.edge()).or_default() += 1;
        }
    }
    let mut angles = Vec::new();
    let mut seam = None;
    let mut heights = (f64::INFINITY, f64::NEG_INFINITY);
    for (edge, count) in uses {
        interrupt::check()?;
        let edge = solid.edge(edge).ok_or(WrapError::Unassembled)?;
        let samples = edge.curve().sample(edge.interval(), tolerance);
        for sample in &samples {
            let (height, angle) = wrap.angle_of(sample.point);
            heights = (heights.0.min(height), heights.1.max(height));
            angles.push(angle.rem_euclid(TAU));
        }
        if count > 1 {
            seam = samples
                .get(samples.len() / 2)
                .map(|sample| wrap.angle_of(sample.point).1.rem_euclid(TAU));
        }
    }
    angles.sort_by(f64::total_cmp);
    let wrapped_gap = match (angles.first(), angles.last()) {
        (Some(first), Some(last)) => Some(Span {
            start: *last,
            length: first + TAU - last,
        }),
        _ => None,
    };
    let gap = angles
        .windows(2)
        .filter_map(|pair| match pair {
            [from, to] => Some(Span {
                start: *from,
                length: to - from,
            }),
            _ => None,
        })
        .chain(wrapped_gap)
        .max_by(|one, other| one.length.total_cmp(&other.length))
        .filter(|gap| seam.is_none() && gap.length > SEAM_TOLERANCE);
    if !heights.0.is_finite() {
        return Err(WrapError::Unassembled);
    }
    Ok(Coverage { seam, gap, heights })
}

struct Window {
    start: f64,
    width: f64,
    full: bool,
    free: bool,
    reach: f64,
}

impl Window {
    fn seam(&self) -> f64 {
        self.start - 0.5 * (TAU - self.width)
    }

    fn reach_start(&self) -> f64 {
        self.start - self.reach
    }
}

fn window(coverages: &[Coverage]) -> Result<Window, WrapError> {
    let seams: Vec<f64> = coverages.iter().filter_map(|found| found.seam).collect();
    let full = |start: f64| Window {
        start,
        width: TAU,
        full: true,
        free: false,
        reach: 0.0,
    };
    if let Some(first) = seams.first() {
        let apart = |angle: f64| {
            let ahead = (angle - first).rem_euclid(TAU);
            ahead.min(TAU - ahead) > SEAM_TOLERANCE
        };
        let refused = seams.iter().any(|seam| apart(*seam))
            || coverages
                .iter()
                .filter(|found| found.seam.is_none())
                .any(|found| !found.gap.is_some_and(|gap| gap.holds(*first)));
        if refused {
            return Err(WrapError::NoSeamPlace);
        }
        return Ok(full(*first));
    }
    let mut common: Option<Span> = None;
    for found in coverages {
        let gap = found.gap.ok_or(WrapError::NoSeamPlace)?;
        common = Some(match common {
            None => gap,
            Some(span) => span.meet(&gap).ok_or(WrapError::NoSeamPlace)?,
        });
    }
    let gap = common.ok_or(WrapError::Misses)?;
    if gap.length < OPEN_GAP {
        return Ok(full(gap.start + 0.5 * gap.length));
    }
    Ok(Window {
        start: gap.start + gap.length * 2.0 / 3.0,
        width: TAU - gap.length / 3.0,
        full: false,
        free: false,
        reach: gap.length / 6.0,
    })
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Role {
    Chain { forward: Option<bool> },
    Start,
    Finish,
    Rim,
}

#[derive(Default)]
struct Sheet {
    curves: Vec<ProfileCurve>,
    roles: BTreeMap<u64, Role>,
}

impl Sheet {
    fn add(&mut self, shape: ProfileShape, role: Role) {
        let entity = self.curves.len() as u64 + 1;
        self.curves.push(ProfileCurve::new(entity, shape));
        self.roles.insert(entity, role);
    }

    fn role(&self, piece: &PieceId) -> Option<Role> {
        self.roles.get(&piece.entity()).copied()
    }
}

fn moved(
    curve: &ProfileCurve,
    map: &impl Fn(Point2) -> Point2,
    rigid: bool,
) -> Result<ProfileShape, WrapError> {
    Ok(match (&curve.shape, rigid) {
        (ProfileShape::Line { start, end }, _) => ProfileShape::Line {
            start: map(*start),
            end: map(*end),
        },
        (
            ProfileShape::Spline {
                degree,
                knots,
                control_points,
                weights,
            },
            _,
        ) => ProfileShape::Spline {
            degree: *degree,
            knots: knots.clone(),
            control_points: control_points.iter().map(|point| map(*point)).collect(),
            weights: weights.clone(),
        },
        (ProfileShape::Arc { center, start, end }, true) => ProfileShape::Arc {
            center: map(*center),
            start: map(*start),
            end: map(*end),
        },
        (
            ProfileShape::EllipticalArc {
                center,
                major,
                minor_radius,
                start,
                end,
            },
            true,
        ) => ProfileShape::EllipticalArc {
            center: map(*center),
            major: *major,
            minor_radius: *minor_radius,
            start: map(*start),
            end: map(*end),
        },
        _ => {
            let (path, range) = curve.curve()?;
            let spline = match &path {
                Curve2::Circle(circle) => arc_spline(circle, range)?,
                Curve2::Ellipse(ellipse) => ellipse_spline(ellipse, range)?,
                _ => return Err(WrapError::Unassembled),
            }
            .map_points(map)?;
            ProfileShape::Spline {
                degree: spline.degree(),
                knots: spline.knots().to_vec(),
                control_points: spline.control_points().to_vec(),
                weights: spline.weights().map(<[f64]>::to_vec),
            }
        }
    })
}

struct Bounds {
    along: (f64, f64),
    reach: (f64, f64),
}

fn bounds(wrap: &Wrap<'_>, heights: (f64, f64)) -> Result<Bounds, WrapError> {
    let (low, high) = heights;
    let ends = [wrap.along_at_height(low), wrap.along_at_height(high)];
    let least = least_radius(wrap, ends.into_iter())?;
    let slope = wrap.radius_slope().abs();
    let mut margin = MARGIN * ((high - low) + least);
    if slope > 0.0 {
        margin = margin.min(APEX_SHARE * least / slope);
    }
    let along = (
        wrap.along_at_height(low - margin),
        wrap.along_at_height(high + margin),
    );
    let reach = (
        wrap.along_at_height(low - 2.0 * margin),
        wrap.along_at_height(high + 2.0 * margin),
    );
    least_radius(wrap, [reach.0, reach.1].into_iter())?;
    Ok(Bounds { along, reach })
}

fn extension(
    wrap: &Wrap<'_>,
    end: &ChainEnd,
    window: &Window,
    bounds: &Bounds,
) -> Result<Option<f64>, WrapError> {
    let (along, round) = wrap.unrolled(end.point);
    let (low, high) = bounds.reach;
    if along <= low || along >= high {
        return Ok(None);
    }
    let rise = end.outward.dot(wrap.axis_in_sketch());
    let turn = end.outward.dot(wrap.across_in_sketch());
    let leaving = if rise > 0.0 {
        (high - along) / rise
    } else if rise < 0.0 {
        (low - along) / rise
    } else {
        f64::INFINITY
    };
    let mut reach = leaving;
    if !window.full {
        let radius = wrap.radius_at(along);
        let angle = round / radius;
        let first = window.reach_start() + TAU * ((angle - window.reach_start()) / TAU).floor();
        for bound in [first, first + TAU] {
            let rate = turn - bound * rise * wrap.radius_slope();
            if rate.abs() > f64::EPSILON {
                let length = (bound * radius - round) / rate;
                if length > LINEAR_RESOLUTION {
                    reach = reach.min(length);
                }
            }
        }
    }
    if reach.is_finite() {
        Ok(Some(reach))
    } else {
        Err(WrapError::EndRunsRound)
    }
}

fn angle_spans(
    wrap: &Wrap<'_>,
    curves: &[(ProfileCurve, Role)],
    tolerance: &SamplingTolerance,
) -> Result<Vec<(f64, f64)>, WrapError> {
    let mut spans = Vec::new();
    for (curve, _) in curves {
        let (path, interval) = curve.curve()?;
        let angles: Vec<f64> = path
            .sample(interval, tolerance)
            .iter()
            .map(|sample| wrap.angle(sample.point))
            .collect();
        spans.extend(angles.windows(2).filter_map(|pair| match pair {
            [from, to] => Some((from.min(*to), from.max(*to))),
            _ => None,
        }));
    }
    if spans.is_empty() {
        Err(WrapError::NotOneChain)
    } else {
        Ok(spans)
    }
}

fn untouched(spans: &[(f64, f64)]) -> Option<f64> {
    let mut pieces: Vec<(f64, f64)> = Vec::with_capacity(spans.len() + 1);
    for (low, high) in spans {
        if high - low >= TAU {
            return None;
        }
        let start = low.rem_euclid(TAU);
        let end = start + (high - low);
        if end > TAU {
            pieces.extend([(start, TAU), (0.0, end - TAU)]);
        } else {
            pieces.push((start, end));
        }
    }
    pieces.sort_by(|one, other| one.0.total_cmp(&other.0));
    let (first, _) = *pieces.first()?;
    let mut reach = first;
    let mut best = Span {
        start: 0.0,
        length: 0.0,
    };
    for (start, end) in &pieces {
        if *start - reach > best.length {
            best = Span {
                start: reach,
                length: start - reach,
            };
        }
        reach = reach.max(*end);
    }
    if first + TAU - reach > best.length {
        best = Span {
            start: reach,
            length: first + TAU - reach,
        };
    }
    (best.length > OPEN_GAP).then_some(best.start + 0.5 * best.length)
}

fn extended(
    wrap: &Wrap<'_>,
    chain: &[ProfileCurve],
    ends: &[ChainEnd; 2],
    window: &Window,
    bounds: &Bounds,
) -> Result<Vec<(ProfileCurve, Role)>, WrapError> {
    let [first, _] = ends;
    let mut curves: Vec<(ProfileCurve, Role)> = chain
        .iter()
        .enumerate()
        .map(|(index, curve)| {
            let forward = (index == first.curve).then_some(!first.at_end);
            (curve.clone(), Role::Chain { forward })
        })
        .collect();
    for (index, end) in ends.iter().enumerate() {
        if let Some(length) = extension(wrap, end, window, bounds)? {
            let forward = (index == 0).then_some(false);
            curves.push((
                ProfileCurve::line(0, end.point, end.point + end.outward * length),
                Role::Chain { forward },
            ));
        }
    }
    Ok(curves)
}

fn sheet(
    wrap: &Wrap<'_>,
    curves: &[(ProfileCurve, Role)],
    spans: &[(f64, f64)],
    window: &Window,
    bounds: &Bounds,
) -> Result<Sheet, WrapError> {
    let (least, most) = spans
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(least, most), span| {
            (least.min(span.0), most.max(span.1))
        });
    let turn_of = |angle: f64| ((angle - window.start) / TAU).floor();
    let (first_turn, last_turn) = (turn_of(least), turn_of(most));
    let turns = last_turn - first_turn;
    if turns.is_nan() || turns >= MAX_TURNS {
        return Err(WrapError::TooIntricate);
    }
    let rigid = wrap.radius_slope() == 0.0;
    let across = wrap.across_in_sketch();
    let mut sheet = Sheet::default();
    let mut turn = first_turn;
    while turn <= last_turn {
        interrupt::check()?;
        let map = |point: Point2| {
            let along = wrap.unrolled(point).0;
            point - across * (TAU * turn * wrap.radius_at(along))
        };
        for (curve, role) in curves {
            sheet.add(moved(curve, &map, rigid)?, *role);
        }
        turn += 1.0;
    }
    let (low, high) = bounds.along;
    let corner = |along: f64, angle: f64| wrap.sketch_point(along, angle * wrap.radius_at(along));
    let (start, finish) = (window.start, window.start + window.width);
    for (from, to, role) in [
        (corner(low, start), corner(low, finish), Role::Rim),
        (corner(low, finish), corner(high, finish), Role::Finish),
        (corner(high, finish), corner(high, start), Role::Rim),
        (corner(high, start), corner(low, start), Role::Start),
    ] {
        sheet.add(
            ProfileShape::Line {
                start: from,
                end: to,
            },
            role,
        );
    }
    Ok(sheet)
}

fn colours(sheet: &Sheet, regions: &[&Region]) -> Result<Vec<bool>, WrapError> {
    let mut sides: BTreeMap<&PieceId, Vec<usize>> = BTreeMap::new();
    for (index, region) in regions.iter().enumerate() {
        for piece in region.pieces() {
            if matches!(sheet.role(piece.id()), Some(Role::Chain { .. })) {
                sides.entry(piece.id()).or_default().push(index);
            }
        }
    }
    let mut colour: Vec<Option<bool>> = vec![None; regions.len()];
    let mut queue = VecDeque::from([0]);
    if let Some(seed) = colour.first_mut() {
        *seed = Some(false);
    }
    while let Some(index) = queue.pop_front() {
        let own = colour
            .get(index)
            .copied()
            .flatten()
            .ok_or(WrapError::Unassembled)?;
        let Some(region) = regions.get(index) else {
            continue;
        };
        for piece in region.pieces() {
            for other in sides.get(piece.id()).into_iter().flatten() {
                if *other == index {
                    continue;
                }
                match colour.get_mut(*other) {
                    Some(slot @ None) => {
                        *slot = Some(!own);
                        queue.push_back(*other);
                    }
                    Some(Some(found)) if *found == own => return Err(WrapError::CrossesItself),
                    _ => {}
                }
            }
        }
    }
    colour
        .into_iter()
        .map(|found| found.ok_or(WrapError::CrossesItself))
        .collect()
}

fn left_colour(sheet: &Sheet, regions: &[&Region], colours: &[bool]) -> bool {
    regions
        .iter()
        .zip(colours)
        .find_map(|(region, colour)| {
            region
                .pieces()
                .find_map(|piece| match sheet.role(piece.id()) {
                    Some(Role::Chain {
                        forward: Some(forward),
                    }) => Some(if piece.is_reversed() != forward {
                        *colour
                    } else {
                        !*colour
                    }),
                    _ => None,
                })
        })
        .unwrap_or(false)
}

fn touches_itself(wrap: &Wrap<'_>, sheet: &Sheet, regions: &[&Region], free: bool) -> bool {
    let reach = |role: Role| -> Vec<(f64, f64)> {
        regions
            .iter()
            .flat_map(|region| region.pieces())
            .filter(|piece| sheet.role(piece.id()) == Some(role))
            .map(|piece| {
                let (one, other) = (wrap.unrolled(piece.start()).0, wrap.unrolled(piece.end()).0);
                (one.min(other), one.max(other))
            })
            .collect()
    };
    let (starts, finishes) = (reach(Role::Start), reach(Role::Finish));
    if free {
        return !starts.is_empty() || !finishes.is_empty();
    }
    starts.iter().any(|start| {
        finishes
            .iter()
            .any(|finish| start.0.max(finish.0) < start.1.min(finish.1) - EDGE_OVERLAP)
    })
}

fn attempt(
    wrap: &Wrap<'_>,
    curves: &[(ProfileCurve, Role)],
    spans: &[(f64, f64)],
    window: &Window,
    bounds: &Bounds,
) -> Result<Vec<Region>, WrapError> {
    let sheet = sheet(wrap, curves, spans, window, bounds)?;
    let profile = Profile::new(&sheet.curves)?;
    let (low, high) = bounds.along;
    let (start, finish) = (window.start, window.start + window.width);
    let mut inside = Vec::new();
    for region in profile.regions() {
        interrupt::check()?;
        let anchor = region.anchor().ok_or(WrapError::Unassembled)?;
        let along = wrap.unrolled(anchor).0;
        let angle = wrap.angle(anchor);
        if along > low && along < high && angle > start && angle < finish {
            inside.push(region);
        }
    }
    let colours = colours(&sheet, &inside)?;
    let left = left_colour(&sheet, &inside, &colours);
    let chosen = |colour: bool| -> Vec<&Region> {
        inside
            .iter()
            .zip(&colours)
            .filter(|(_, found)| **found == colour)
            .map(|(region, _)| *region)
            .collect()
    };
    let selected = [left, !left]
        .into_iter()
        .map(chosen)
        .find(|regions| !window.full || !touches_itself(wrap, &sheet, regions, window.free))
        .ok_or(WrapError::ClosesRound)?;
    if selected.is_empty() || selected.len() == inside.len() {
        return Err(WrapError::Misses);
    }
    Ok(selected.into_iter().cloned().collect())
}

pub fn wrap_chain(
    plane: &Plane,
    curves: &[ProfileCurve],
    surface: &Surface,
    solid: &Solid,
    faces: &[FaceId],
    feature: u64,
) -> Result<Solid, WrapError> {
    let ends = chain_ends(curves)?;
    let extent = curves
        .iter()
        .filter_map(|curve| curve.curve().ok())
        .map(|(path, range)| path.bounding_box(range).size().length())
        .fold(0.0, f64::max);
    let rough = SamplingTolerance::for_extent(extent.max(LINEAR_RESOLUTION));
    let mut points = Vec::new();
    for curve in curves {
        let (path, range) = curve.curve()?;
        points.extend(
            path.sample(range, &rough)
                .into_iter()
                .map(|sample| sample.point),
        );
    }
    let wrap = Wrap::new(plane, surface)?.referenced(&points)?;
    let tolerance = SamplingTolerance::new(MARGIN * extent.max(LINEAR_RESOLUTION), SAMPLING_ANGLE)
        .ok_or(WrapError::Unassembled)?;
    let coverages = faces
        .iter()
        .map(|face| coverage(&wrap, solid, *face, &tolerance))
        .collect::<Result<Vec<_>, _>>()?;
    let window = window(&coverages)?;
    let heights = coverages
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), found| {
            (low.min(found.heights.0), high.max(found.heights.1))
        });
    let bounds = bounds(&wrap, heights)?;
    let extended = extended(&wrap, curves, &ends, &window, &bounds)?;
    let spans = angle_spans(&wrap, &extended, &tolerance)?;
    let mut windows = vec![window];
    if let Some(start) = windows
        .first()
        .filter(|window| window.full)
        .and_then(|_| untouched(&spans))
    {
        windows.push(Window {
            start,
            width: TAU,
            full: true,
            free: true,
            reach: 0.0,
        });
    }
    let mut outcome = Err(WrapError::ClosesRound);
    for window in &windows {
        outcome =
            attempt(&wrap, &extended, &spans, window, &bounds).map(|regions| (regions, window));
        if !matches!(outcome, Err(WrapError::ClosesRound)) {
            break;
        }
    }
    let (regions, window) = outcome?;
    let (low, high) = bounds.along;
    let wrapping = Wrapping {
        seam: window.seam(),
        least_radius: least_radius(&wrap, [low, high].into_iter())?,
    };
    wrapped(&wrap, &regions, &wrapping, feature)
}
