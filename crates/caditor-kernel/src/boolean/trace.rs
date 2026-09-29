use std::{cmp::Ordering, collections::BTreeMap, f64::consts::TAU};

use caditor_geometry::{Aabb2, Point2, Point3, Vector2, Vector3};

use crate::{
    boolean::{BooleanError, imprint::Arrangement},
    sense::Sense,
    surface::Surface,
    topology::{Pcurve, continues, fit_pcurve, inside_polygon, signed_area},
};

const ANGLE_TIE: f64 = 1e-5;
const CHORD_FRACTION: f64 = 0.01;
const SCAN_LEVELS: usize = 24;
const PERIOD_SHIFTS: [f64; 3] = [0.0, -1.0, 1.0];
const JOINT_MATCH: f64 = 1e-6;
const INTERIOR_POINTS: usize = 3;
const SPAN_OFFSETS: [f64; 3] = [0.5, 0.37, 0.61];
const POLE_RING: usize = 8;
const POLE_OFFSET: f64 = 1e-3;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct HalfEdge {
    pub piece: usize,
    pub sense: Sense,
    pub hint: Option<Point2>,
}

impl HalfEdge {
    pub fn new(piece: usize, sense: Sense, hint: Option<Point2>) -> Self {
        Self { piece, sense, hint }
    }

    fn is_reverse_of(&self, other: &Self) -> bool {
        self.piece == other.piece && self.sense != other.sense
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Chart<'a> {
    pub surface: &'a Surface,
    pub sense: Sense,
    pub center: Point2,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Coedge {
    pub half_edge: HalfEdge,
    pub pcurve: Pcurve,
}

impl Coedge {
    #[must_use]
    pub fn reversed(&self) -> Self {
        Self {
            half_edge: HalfEdge {
                piece: self.half_edge.piece,
                sense: self.half_edge.sense.reversed(),
                hint: Some(self.pcurve.end()),
            },
            pcurve: self.pcurve.reversed(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct TracedLoop {
    pub coedges: Vec<Coedge>,
    pub area: f64,
}

impl TracedLoop {
    pub fn polygon(&self) -> Vec<Point2> {
        self.coedges
            .iter()
            .flat_map(|coedge| coedge.pcurve.samples().iter().map(|sample| sample.uv))
            .collect()
    }

    #[must_use]
    pub fn reversed(&self) -> Self {
        Self {
            coedges: self.coedges.iter().rev().map(Coedge::reversed).collect(),
            area: -self.area,
        }
    }

    fn shifted(self, offset: Vector2) -> Self {
        Self {
            coedges: self
                .coedges
                .into_iter()
                .map(|coedge| Coedge {
                    half_edge: HalfEdge {
                        hint: coedge.half_edge.hint.map(|hint| hint + offset),
                        ..coedge.half_edge
                    },
                    pcurve: coedge.pcurve.shifted(offset),
                })
                .collect(),
            area: self.area,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Fragment {
    pub loops: Vec<TracedLoop>,
}

impl Fragment {
    #[must_use]
    pub fn reversed(&self) -> Self {
        Self {
            loops: self.loops.iter().map(TracedLoop::reversed).collect(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Travel {
    start: usize,
    end: usize,
    leaving: Vector3,
    arriving: Vector3,
    length: f64,
    piece: usize,
    sense: Sense,
}

fn travel(arrangement: &Arrangement, half_edge: &HalfEdge) -> Option<Travel> {
    let (curve, piece) = arrangement.curve(half_edge.piece)?;
    let sign = half_edge.sense.sign();
    let (from, to) = if half_edge.sense.is_same() {
        (piece.interval.start(), piece.interval.end())
    } else {
        (piece.interval.end(), piece.interval.start())
    };
    Some(Travel {
        start: piece.start_of(half_edge.sense),
        end: piece.end_of(half_edge.sense),
        leaving: curve.evaluate(from).first * sign,
        arriving: curve.evaluate(to).first * sign,
        length: curve.length(piece.interval),
        piece: half_edge.piece,
        sense: half_edge.sense,
    })
}

fn point_along(
    arrangement: &Arrangement,
    travel: &Travel,
    from_start: bool,
    distance: f64,
) -> Option<Point3> {
    let (curve, piece) = arrangement.curve(travel.piece)?;
    let forward = travel.sense.is_same() == from_start;
    let (origin, direction) = if forward {
        (piece.interval.start(), 1.0)
    } else {
        (piece.interval.end(), -1.0)
    };
    let speed = curve.evaluate(origin).first.length();
    let step = if speed > 0.0 {
        (distance / speed).min(piece.interval.length())
    } else {
        0.5 * piece.interval.length()
    };
    Some(curve.point(origin + direction * step))
}

struct Frame {
    x: Vector3,
    y: Vector3,
}

impl Frame {
    fn new(normal: Vector3, reference: Vector3) -> Option<Self> {
        let x = (reference - normal * normal.dot(reference)).try_normalize()?;
        Some(Self {
            x,
            y: normal.cross(x),
        })
    }

    fn signed_angle(&self, direction: Vector3) -> f64 {
        direction.dot(self.y).atan2(direction.dot(self.x))
    }

    fn angle(&self, direction: Vector3) -> f64 {
        let angle = self.signed_angle(direction);
        if angle < 0.0 { angle + TAU } else { angle }
    }
}

struct Turn {
    primary: f64,
    chord: f64,
}

impl Turn {
    fn new(frame: &Frame, leaving: Vector3, chord: Vector3, reference: f64) -> Self {
        let tangent = frame.angle(leaving);
        let chord_angle = frame.signed_angle(chord);
        let primary = if tangent < ANGLE_TIE || tangent > TAU - ANGLE_TIE {
            let turn = chord_angle - reference;
            if turn > 0.0 { turn } else { TAU + turn }
        } else {
            tangent
        };
        Self {
            primary,
            chord: if chord_angle < 0.0 {
                chord_angle + TAU
            } else {
                chord_angle
            },
        }
    }

    fn compare(&self, other: &Self) -> Ordering {
        if (self.primary - other.primary).abs() > ANGLE_TIE {
            self.primary.total_cmp(&other.primary)
        } else {
            self.chord.total_cmp(&other.chord)
        }
    }
}

fn vertex_normal(
    arrangement: &Arrangement,
    chart: &Chart,
    vertex: usize,
    hint: Option<Point2>,
) -> Option<Vector3> {
    let point = arrangement.point(vertex)?;
    let surface = chart.surface;
    let uv = surface.project(point, Some(hint.unwrap_or(chart.center)));
    let normal = match surface.pole_at(uv) {
        Some(pole) => pole_normal(surface, pole.v)?,
        None => surface.normal(uv.x, uv.y)?,
    };
    Some(normal * chart.sense.sign())
}

fn pole_normal(surface: &Surface, pole: f64) -> Option<Vector3> {
    let domain = surface.v_domain();
    let span = domain.end() - domain.start();
    let offset = if span.is_finite() {
        POLE_OFFSET.min(span * POLE_OFFSET)
    } else {
        POLE_OFFSET
    };
    let inward = if (pole - domain.start()).abs() <= (pole - domain.end()).abs() {
        offset
    } else {
        -offset
    };
    let period = surface.u_period().unwrap_or(TAU);
    let start = surface.u_domain().start();
    let start = if start.is_finite() { start } else { 0.0 };
    let sum = (0..POLE_RING)
        .filter_map(|index| {
            let u = start + period * index as f64 / POLE_RING as f64;
            surface.normal(u, pole + inward)
        })
        .fold(Vector3::ZERO, |sum, normal| sum + normal);
    sum.try_normalize()
}

pub(super) fn trace(
    arrangement: &Arrangement,
    chart: &Chart,
    half_edges: &[HalfEdge],
) -> Result<Vec<Vec<HalfEdge>>, BooleanError> {
    let travels = half_edges
        .iter()
        .map(|half_edge| travel(arrangement, half_edge))
        .collect::<Option<Vec<Travel>>>()
        .ok_or(BooleanError::Split)?;
    let mut leaving: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (index, travel) in travels.iter().enumerate() {
        leaving.entry(travel.start).or_default().push(index);
    }
    let mut normals: BTreeMap<usize, Vector3> = BTreeMap::new();
    for (vertex, members) in &leaving {
        let hint = members
            .iter()
            .find_map(|index| half_edges.get(*index).and_then(|half_edge| half_edge.hint));
        if let Some(normal) = vertex_normal(arrangement, chart, *vertex, hint) {
            normals.insert(*vertex, normal);
        }
    }
    let next = |arriving: usize| -> Option<usize> {
        let travel = travels.get(arriving)?;
        let current = half_edges.get(arriving)?;
        let candidates: Vec<usize> = leaving.get(&travel.end)?.clone();
        let forward: Vec<usize> = candidates
            .iter()
            .copied()
            .filter(|index| {
                half_edges
                    .get(*index)
                    .is_some_and(|candidate| !candidate.is_reverse_of(current))
            })
            .collect();
        match forward.as_slice() {
            [] => return candidates.first().copied(),
            [only] => return Some(*only),
            _ => {}
        }
        let normal = normals.get(&travel.end)?;
        let frame = Frame::new(*normal, -travel.arriving)?;
        let point = arrangement.point(travel.end)?;
        let reach = CHORD_FRACTION
            * forward
                .iter()
                .filter_map(|index| travels.get(*index))
                .map(|candidate| candidate.length)
                .fold(travel.length, f64::min);
        let back = point_along(arrangement, travel, false, reach)? - point;
        let reference = frame.signed_angle(back);
        let turns: Vec<(usize, Turn)> = forward
            .iter()
            .filter_map(|index| {
                let candidate = travels.get(*index)?;
                let chord = point_along(arrangement, candidate, true, reach)? - point;
                Some((
                    *index,
                    Turn::new(&frame, candidate.leaving, chord, reference),
                ))
            })
            .collect();
        turns
            .iter()
            .max_by(|a, b| a.1.compare(&b.1))
            .map(|(index, _)| *index)
    };
    let mut used = vec![false; half_edges.len()];
    let mut loops = Vec::new();
    for start in 0..half_edges.len() {
        if used.get(start).copied().unwrap_or(true) {
            continue;
        }
        let mut members = Vec::new();
        let mut current = start;
        loop {
            match used.get_mut(current) {
                Some(flag) if !*flag => *flag = true,
                _ => return Err(BooleanError::Split),
            }
            members.push(*half_edges.get(current).ok_or(BooleanError::Split)?);
            current = next(current).ok_or(BooleanError::Split)?;
            if current == start {
                break;
            }
        }
        loops.push(members);
    }
    Ok(loops)
}

pub(super) fn fit_loop(
    arrangement: &Arrangement,
    chart: &Chart,
    mut members: Vec<HalfEdge>,
) -> Result<TracedLoop, BooleanError> {
    let first_hinted = members
        .iter()
        .position(|half_edge| half_edge.hint.is_some())
        .unwrap_or(0);
    members.rotate_left(first_hinted);
    let hinted: Vec<bool> = members
        .iter()
        .map(|half_edge| half_edge.hint.is_some())
        .collect();
    let mut coedges: Vec<Coedge> = Vec::with_capacity(members.len());
    for half_edge in members {
        let (curve, piece) = arrangement
            .curve(half_edge.piece)
            .ok_or(BooleanError::Split)?;
        let previous = coedges.last().map(|coedge| coedge.pcurve.end());
        let hint = half_edge.hint.or(previous).unwrap_or(chart.center);
        let pcurve = fit_pcurve(
            chart.surface,
            curve,
            piece.interval,
            half_edge.sense,
            Some(hint),
        )
        .map_err(|_| BooleanError::Split)?;
        coedges.push(Coedge {
            half_edge: HalfEdge {
                hint: Some(pcurve.start()),
                ..half_edge
            },
            pcurve,
        });
    }
    place_runs_from_poles(chart, &hinted, &mut coedges);
    snap_ends(arrangement, chart.surface, &mut coedges);
    let count = coedges.len();
    for (index, coedge) in coedges.iter().enumerate() {
        let next = coedges
            .get((index + 1) % count)
            .ok_or(BooleanError::Split)?;
        if !continues(chart.surface, coedge.pcurve.end(), next.pcurve.start()) {
            return Err(BooleanError::Split);
        }
    }
    let mut traced = TracedLoop { coedges, area: 0.0 };
    traced.area = signed_area(&traced.polygon());
    if !traced.area.is_finite() || traced.area == 0.0 {
        return Err(BooleanError::Split);
    }
    Ok(traced)
}

fn vertex_uv(
    arrangement: &Arrangement,
    surface: &Surface,
    vertex: Option<usize>,
    near: Point2,
) -> Point2 {
    if surface.pole_at(near).is_some() {
        return near;
    }
    vertex
        .and_then(|vertex| arrangement.point(vertex))
        .map_or(near, |point| surface.project(point, Some(near)))
}

pub(super) fn snap_ends(arrangement: &Arrangement, surface: &Surface, coedges: &mut [Coedge]) {
    for coedge in coedges.iter_mut() {
        let piece = arrangement.piece(coedge.half_edge.piece);
        let start = vertex_uv(
            arrangement,
            surface,
            piece.map(|piece| piece.start_of(coedge.half_edge.sense)),
            coedge.pcurve.start(),
        );
        let end = vertex_uv(
            arrangement,
            surface,
            piece.map(|piece| piece.end_of(coedge.half_edge.sense)),
            coedge.pcurve.end(),
        );
        coedge.pcurve = coedge.pcurve.with_ends(start, end);
        coedge.half_edge.hint = Some(start);
    }
    let count = coedges.len();
    for index in 0..count {
        let Some(end) = coedges.get(index).map(|coedge| coedge.pcurve.end()) else {
            continue;
        };
        if let Some(next) = coedges.get_mut((index + 1) % count)
            && next.pcurve.start() != end
            && (next.pcurve.start() - end).abs().max_element() <= JOINT_MATCH
        {
            next.pcurve = next.pcurve.with_ends(end, next.pcurve.end());
            next.half_edge.hint = Some(end);
        }
    }
}

fn place_runs_from_poles(chart: &Chart, hinted: &[bool], coedges: &mut [Coedge]) {
    let Some(period) = chart.surface.u_period() else {
        return;
    };
    let surface = chart.surface;
    let at_pole = |coedge: &Coedge| surface.pole_at(coedge.pcurve.start()).is_some();
    let count = coedges.len();
    let mut index = 0;
    while index < count {
        let free =
            !hinted.get(index).copied().unwrap_or(true) && coedges.get(index).is_some_and(at_pole);
        if !free {
            index += 1;
            continue;
        }
        let mut end = index + 1;
        while end < count
            && !hinted.get(end).copied().unwrap_or(true)
            && !coedges.get(end).is_some_and(at_pole)
        {
            end += 1;
        }
        let following = coedges
            .get(end % count)
            .filter(|coedge| hinted.get(end % count).copied().unwrap_or(false) && !at_pole(coedge));
        let (reference, target) = match (following, coedges.get(end - 1)) {
            (Some(following), Some(last)) => (last.pcurve.end().x, following.pcurve.start().x),
            (_, Some(last)) => (
                0.5 * (last.pcurve.end().x
                    + coedges.get(index).map_or(0.0, |c| c.pcurve.start().x)),
                chart.center.x,
            ),
            _ => break,
        };
        let offset = Vector2::new(((target - reference) / period).round() * period, 0.0);
        if offset.x != 0.0 {
            for coedge in coedges.iter_mut().take(end).skip(index) {
                coedge.pcurve = coedge.pcurve.shifted(offset);
                coedge.half_edge.hint = Some(coedge.pcurve.start());
            }
        }
        index = end;
    }
}

fn shifts(surface: &Surface) -> Vec<Vector2> {
    let along = |period: Option<f64>| match period {
        Some(period) => PERIOD_SHIFTS.iter().map(|shift| shift * period).collect(),
        None => vec![0.0],
    };
    let (u, v) = (along(surface.u_period()), along(surface.v_period()));
    u.iter()
        .flat_map(|du| v.iter().map(move |dv| Vector2::new(*du, *dv)))
        .collect()
}

fn probe_outside(hole: &TracedLoop, outer: &TracedLoop) -> Option<Point2> {
    let pieces: Vec<usize> = outer
        .coedges
        .iter()
        .map(|coedge| coedge.half_edge.piece)
        .collect();
    hole.coedges
        .iter()
        .find(|coedge| !pieces.contains(&coedge.half_edge.piece))
        .map(|coedge| {
            let samples = coedge.pcurve.samples();
            match samples {
                [first, second] => first.uv.lerp(second.uv, 0.5),
                _ => samples
                    .get(samples.len() / 2)
                    .map_or(coedge.pcurve.start(), |sample| sample.uv),
            }
        })
}

pub(super) fn group(chart: &Chart, loops: Vec<TracedLoop>) -> Result<Vec<Fragment>, BooleanError> {
    let sign = chart.sense.sign();
    let (outers, holes): (Vec<TracedLoop>, Vec<TracedLoop>) = loops
        .into_iter()
        .partition(|traced| traced.area * sign > 0.0);
    let mut fragments: Vec<Fragment> = outers
        .into_iter()
        .map(|outer| Fragment { loops: vec![outer] })
        .collect();
    let offsets = shifts(chart.surface);
    let mut placed: Vec<(usize, TracedLoop)> = Vec::new();
    for hole in holes {
        let mut best: Option<(usize, f64, Vector2)> = None;
        for (index, fragment) in fragments.iter().enumerate() {
            let Some(outer) = fragment.loops.first() else {
                continue;
            };
            let Some(probe) = probe_outside(&hole, outer) else {
                continue;
            };
            let polygon = outer.polygon();
            let Some(offset) = offsets
                .iter()
                .copied()
                .find(|offset| inside_polygon(&polygon, probe + *offset))
            else {
                continue;
            };
            let area = outer.area.abs();
            if best.is_none_or(|(_, smallest, _)| area < smallest) {
                best = Some((index, area, offset));
            }
        }
        let (index, _, offset) = best.ok_or(BooleanError::Split)?;
        placed.push((index, hole.shifted(offset)));
    }
    for (index, hole) in placed {
        if let Some(fragment) = fragments.get_mut(index) {
            fragment.loops.push(hole);
        }
    }
    Ok(fragments)
}

fn scan(polygons: &[Vec<Point2>], level: f64, across: bool) -> Vec<f64> {
    let along = |point: Point2| if across { point.y } else { point.x };
    let height = |point: Point2| if across { point.x } else { point.y };
    let mut crossings = Vec::new();
    for polygon in polygons {
        let count = polygon.len();
        for (index, a) in polygon.iter().enumerate() {
            let Some(b) = polygon.get((index + 1) % count) else {
                continue;
            };
            let (ha, hb) = (height(*a), height(*b));
            if (ha > level) != (hb > level) {
                let fraction = (level - ha) / (hb - ha);
                crossings.push(along(*a) + fraction * (along(*b) - along(*a)));
            }
        }
    }
    crossings.sort_by(f64::total_cmp);
    crossings
}

pub(super) fn interior_points(fragment: &Fragment, surface: &Surface) -> Vec<Point2> {
    let polygons: Vec<Vec<Point2>> = fragment.loops.iter().map(TracedLoop::polygon).collect();
    let Some(bounds) = polygons
        .first()
        .and_then(|outer| Aabb2::from_points(outer.iter().copied()))
    else {
        return Vec::new();
    };
    let mut spans: Vec<(f64, Point2)> = Vec::new();
    for across in [false, true] {
        let (low, high) = if across {
            (bounds.min().x, bounds.max().x)
        } else {
            (bounds.min().y, bounds.max().y)
        };
        for step in 0..SCAN_LEVELS {
            let fraction = (step as f64 + 0.5 + 0.1 * (step % 3) as f64) / SCAN_LEVELS as f64;
            let level = low + (high - low) * fraction;
            let crossings = scan(&polygons, level, across);
            let offset = SPAN_OFFSETS
                .get(step % SPAN_OFFSETS.len())
                .copied()
                .unwrap_or(0.5);
            for [a, b] in crossings.as_chunks::<2>().0 {
                let along = a + (b - a) * offset;
                let uv = if across {
                    Point2::new(level, along)
                } else {
                    Point2::new(along, level)
                };
                let derivatives = surface.evaluate(uv.x, uv.y);
                let speed = if across {
                    derivatives.dv.length()
                } else {
                    derivatives.du.length()
                };
                let width = (b - a) * speed;
                if width.is_finite() && width > 0.0 {
                    spans.push((width, uv));
                }
            }
        }
    }
    spans.sort_by(|a, b| b.0.total_cmp(&a.0));
    spans
        .into_iter()
        .take(INTERIOR_POINTS)
        .map(|(_, uv)| uv)
        .collect()
}
