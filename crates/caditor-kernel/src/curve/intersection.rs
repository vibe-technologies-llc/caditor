use std::{
    ops::Range,
    sync::{Arc, OnceLock},
};

use caditor_geometry::{Aabb, Point2, Point3, Similarity, Vector3};

use crate::{
    box_tree::BoxTree,
    coordinates::angle_between,
    error::GeometryError,
    interrupt,
    intersect::solve::{Contact, contact_direction, refine_on_plane},
    interval::Interval,
    numeric::integrate,
    surface::{Surface, periodic_near},
    tolerance::{INTERSECTION_TOLERANCE, LINEAR_RESOLUTION},
};

const MAX_NODES: usize = 1 << 16;
const MAX_REFINE_DEPTH: usize = 40;
const MAX_SEGMENT_TURN: f64 = 0.35;
const MAX_UNROLLED_PERIODS: i64 = 8;
const SMALL_TURN: f64 = 1e-6;
const TOUCHING_ITERATIONS: usize = 12;
const TOUCHING_GAP: f64 = LINEAR_RESOLUTION;
const SEED_GAP: f64 = 1e-3 * LINEAR_RESOLUTION;
const MIN_INDEXED_SEGMENTS: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IntersectionNode {
    pub parameter: f64,
    pub point: Point3,
    pub derivative: Vector3,
    pub uv: [Point2; 2],
}

#[derive(Debug, Clone)]
pub struct IntersectionCurve {
    surfaces: Arc<[Surface; 2]>,
    nodes: Arc<[IntersectionNode]>,
    closed: bool,
    index: Arc<NodeIndex>,
}

impl PartialEq for IntersectionCurve {
    fn eq(&self, other: &Self) -> bool {
        self.surfaces == other.surfaces && self.nodes == other.nodes && self.closed == other.closed
    }
}

#[derive(Debug, Default)]
struct NodeIndex {
    travelled: OnceLock<Box<[f64]>>,
    hulls: OnceLock<BoxTree>,
}

impl NodeIndex {
    fn heap_size(&self) -> usize {
        size_of::<Self>()
            + self
                .travelled
                .get()
                .map_or(0, |travelled| size_of_val(&**travelled))
            + self.hulls.get().map_or(0, BoxTree::heap_size)
    }
}

struct Window {
    turn: i64,
    segments: Range<usize>,
}

fn finite_node(node: &IntersectionNode) -> bool {
    let [first, second] = node.uv;
    node.parameter.is_finite()
        && node.point.is_finite()
        && node.derivative.is_finite()
        && first.is_finite()
        && second.is_finite()
}

pub(crate) fn hermite(
    start: &IntersectionNode,
    end: &IntersectionNode,
    parameter: f64,
) -> [Point3; 3] {
    let h = end.parameter - start.parameter;
    if h <= 0.0 || !h.is_finite() {
        return [start.point, start.derivative, Vector3::ZERO];
    }
    let s = (parameter - start.parameter) / h;
    let s2 = s * s;
    let s3 = s2 * s;
    let (p0, d0, p1, d1) = (start.point, start.derivative, end.point, end.derivative);
    let point = p0 * (2.0 * s3 - 3.0 * s2 + 1.0)
        + d0 * (h * (s3 - 2.0 * s2 + s))
        + p1 * (3.0 * s2 - 2.0 * s3)
        + d1 * (h * (s3 - s2));
    let first = (p0 * (6.0 * s2 - 6.0 * s) + p1 * (6.0 * s - 6.0 * s2)) / h
        + d0 * (3.0 * s2 - 4.0 * s + 1.0)
        + d1 * (3.0 * s2 - 2.0 * s);
    let second = (p0 * (12.0 * s - 6.0) + p1 * (6.0 - 12.0 * s)) / (h * h)
        + (d0 * (6.0 * s - 4.0) + d1 * (6.0 * s - 2.0)) / h;
    [point, first, second]
}

fn segment_length(start: &IntersectionNode, end: &IntersectionNode, low: f64, high: f64) -> f64 {
    integrate(&[low, 0.5 * (low + high), high], |parameter| {
        let [_, first, _] = hermite(start, end, parameter);
        first.length()
    })
}

fn segment_hull(start: &IntersectionNode, end: &IntersectionNode) -> Aabb {
    let third = (end.parameter - start.parameter) / 3.0;
    Aabb::from_point(start.point)
        .including(start.point + start.derivative * third)
        .including(end.point - end.derivative * third)
        .including(end.point)
}

fn arc_estimate(start: Point3, start_tangent: Vector3, end: Point3, end_tangent: Vector3) -> f64 {
    let chord = start.distance(end);
    let turn = angle_between(start_tangent, end_tangent);
    if turn < SMALL_TURN {
        chord
    } else {
        chord * (0.5 * turn) / (0.5 * turn).sin()
    }
}

#[derive(Debug, Clone, Copy)]
struct Traced {
    contact: Contact,
    tangent: Vector3,
}

impl IntersectionCurve {
    pub(crate) fn heap_size(&self) -> usize {
        size_of_val(&*self.surfaces)
            + self.surfaces.iter().map(Surface::heap_size).sum::<usize>()
            + size_of_val(&*self.nodes)
            + self.index.heap_size()
    }

    pub fn new(
        surfaces: [Surface; 2],
        nodes: Vec<IntersectionNode>,
        closed: bool,
    ) -> Result<Self, GeometryError> {
        if nodes.len() < 2 {
            return Err(GeometryError::TooFewControlPoints {
                degree: 3,
                points: nodes.len(),
            });
        }
        if !nodes.iter().all(finite_node) {
            return Err(GeometryError::NonFinite);
        }
        if nodes
            .windows(2)
            .any(|pair| matches!(pair, [a, b] if b.parameter <= a.parameter))
        {
            return Err(GeometryError::Knots);
        }
        if closed
            && let (Some(first), Some(last)) = (nodes.first(), nodes.last())
            && first.point.distance(last.point) > LINEAR_RESOLUTION
        {
            return Err(GeometryError::DegenerateSurface);
        }
        Ok(Self {
            surfaces: Arc::new(surfaces),
            nodes: nodes.into(),
            closed,
            index: Arc::default(),
        })
    }

    pub(crate) fn from_contacts(
        surfaces: [Surface; 2],
        contacts: &[Contact],
        closed: bool,
    ) -> Option<Self> {
        Self::traced(surfaces, contacts, closed, Touching::Refuse)
    }

    fn traced(
        surfaces: [Surface; 2],
        contacts: &[Contact],
        closed: bool,
        touching: Touching,
    ) -> Option<Self> {
        let traced = {
            let pair = {
                let [a, b] = &surfaces;
                [a, b]
            };
            let mut traced: Vec<Traced> = Vec::with_capacity(contacts.len());
            for (index, contact) in contacts.iter().enumerate() {
                let previous = index
                    .checked_sub(1)
                    .and_then(|before| contacts.get(before))
                    .unwrap_or(contact);
                let next = contacts.get(index + 1).unwrap_or(contact);
                let travel = next.point - previous.point;
                let tangent = contact_direction(pair, contact)
                    .filter(|(_, sine)| *sine > 1e-9)
                    .map_or(travel.normalize_or_zero(), |(direction, _)| direction);
                let tangent = if tangent.dot(travel) < 0.0 {
                    -tangent
                } else {
                    tangent
                };
                traced.push(Traced {
                    contact: *contact,
                    tangent,
                });
            }
            traced.dedup_by(|b, a| a.contact.point.distance(b.contact.point) <= 1e-12);
            refine_traced(pair, &traced, touching)?
        };
        let mut nodes = Vec::with_capacity(traced.len());
        let mut parameter = 0.0;
        for (index, item) in traced.iter().enumerate() {
            if let Some(previous) = index.checked_sub(1).and_then(|before| traced.get(before)) {
                parameter += arc_estimate(
                    previous.contact.point,
                    previous.tangent,
                    item.contact.point,
                    item.tangent,
                );
            }
            nodes.push(IntersectionNode {
                parameter,
                point: item.contact.point,
                derivative: item.tangent,
                uv: item.contact.uv,
            });
        }
        nodes.dedup_by(|b, a| b.parameter <= a.parameter);
        Self::new(surfaces, nodes, closed).ok()
    }

    pub fn through(surfaces: [Surface; 2], points: &[Point3], closed: bool) -> Option<Self> {
        let mut contacts: Vec<Contact> = Vec::with_capacity(points.len());
        {
            let [first, second] = &surfaces;
            let pair = [first, second];
            let mut hints: Option<[Point2; 2]> = None;
            for (index, point) in points.iter().enumerate() {
                let previous = index
                    .checked_sub(1)
                    .and_then(|before| points.get(before))
                    .unwrap_or(point);
                let next = points.get(index + 1).unwrap_or(point);
                let tangent = (*next - *previous).try_normalize()?;
                let start = [
                    first.project(*point, hints.map(|[hint, _]| hint)),
                    second.project(*point, hints.map(|[_, hint]| hint)),
                ];
                let contact = refine_on_plane(pair, start, *point, tangent)
                    .or_else(|| touching_contact(pair, start, *point))?;
                hints = Some(contact.uv);
                contacts.push(contact);
            }
        }
        if closed
            && let (Some(first), Some(last)) = (contacts.first().copied(), contacts.last_mut())
        {
            *last = first;
        }
        Self::traced(surfaces, &contacts, closed, Touching::Follow)
    }

    pub fn surfaces(&self) -> &[Surface; 2] {
        &self.surfaces
    }

    pub fn nodes(&self) -> &[IntersectionNode] {
        &self.nodes
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }

    pub fn domain(&self) -> Interval {
        let start = self.nodes.first().map_or(0.0, |node| node.parameter);
        let end = self.nodes.last().map_or(start, |node| node.parameter);
        Interval::new(start, end).unwrap_or(Interval::UNIT)
    }

    pub fn period(&self) -> Option<f64> {
        self.closed.then(|| self.domain().length())
    }

    fn wrapped(&self, parameter: f64) -> f64 {
        let domain = self.domain();
        match self.period() {
            Some(period) if period > 0.0 && parameter.is_finite() => {
                domain.start() + (parameter - domain.start()).rem_euclid(period)
            }
            _ => domain.clamp(parameter),
        }
    }

    fn segment_index(&self, parameter: f64) -> usize {
        self.nodes
            .partition_point(|node| node.parameter <= parameter)
            .min(self.nodes.len().saturating_sub(1))
            .saturating_sub(1)
    }

    fn segment(&self, index: usize) -> Option<(&IntersectionNode, &IntersectionNode)> {
        Some((self.nodes.get(index)?, self.nodes.get(index + 1)?))
    }

    fn segment_at(&self, parameter: f64) -> Option<(&IntersectionNode, &IntersectionNode)> {
        self.segment(self.segment_index(parameter))
    }

    fn segments(&self) -> impl Iterator<Item = (&IntersectionNode, &IntersectionNode)> {
        self.nodes.iter().zip(self.nodes.iter().skip(1))
    }

    fn travelled(&self) -> &[f64] {
        self.index.travelled.get_or_init(|| {
            let mut sum = 0.0;
            std::iter::once(0.0)
                .chain(self.segments().map(|(start, end)| {
                    sum += segment_length(start, end, start.parameter, end.parameter);
                    sum
                }))
                .collect()
        })
    }

    fn hulls(&self) -> &BoxTree {
        self.index.hulls.get_or_init(|| {
            BoxTree::new(self.segments().map(|(start, end)| segment_hull(start, end)))
        })
    }

    fn length_within(&self, low: f64, high: f64) -> f64 {
        let partial = |index: usize, from: f64, to: f64| match self.segment(index) {
            Some((start, end)) if from < to => segment_length(start, end, from, to),
            _ => 0.0,
        };
        let parameter = |index: usize| self.nodes.get(index).map_or(low, |node| node.parameter);
        let first = self.segment_index(low);
        let last = self.segment_index(high);
        if first >= last {
            return partial(first, low, high);
        }
        let travelled = self.travelled();
        let between = match (travelled.get(first + 1), travelled.get(last)) {
            (Some(from), Some(to)) => to - from,
            _ => 0.0,
        };
        partial(first, low, parameter(first + 1)) + between + partial(last, parameter(last), high)
    }

    pub(crate) fn length(&self, range: Interval) -> f64 {
        let domain = self.domain();
        let Some(period) = self.period().filter(|period| *period > 0.0) else {
            let low = domain.clamp(range.start());
            return self.length_within(low, domain.clamp(range.end()).max(low));
        };
        let turns = ((range.start() - domain.start()) / period).floor();
        let shift = if turns.is_finite() {
            turns * period
        } else {
            0.0
        };
        let low = domain.clamp(range.start() - shift);
        let high = range.end() - shift;
        if high <= domain.end() {
            return self.length_within(low, domain.clamp(high).max(low));
        }
        let beyond = high - domain.end();
        let laps = (beyond / period).floor();
        let rest = domain.clamp(domain.start() + beyond - laps * period);
        let lap = self.travelled().last().copied().unwrap_or(0.0);
        self.length_within(low, domain.end())
            + laps * lap
            + self.length_within(domain.start(), rest)
    }

    pub(crate) fn is_longer_than(&self, range: Interval, bound: f64) -> bool {
        self.point(range.start()).distance(self.point(range.end())) > bound
            || self.length(range) > bound
    }

    pub(crate) fn nearby_runs(&self, point: Point3, range: Interval) -> Vec<Interval> {
        let (range, windows) = self.windows(range);
        let segments: usize = windows.iter().map(|window| window.segments.len()).sum();
        if segments < MIN_INDEXED_SEGMENTS {
            return vec![range];
        }
        let within = |index: usize| {
            windows
                .iter()
                .any(|window| window.segments.contains(&index))
        };
        let nearest = self
            .hulls()
            .possibly_nearest_among(point, LINEAR_RESOLUTION, within);
        let period = self.domain().length();
        let mut runs: Vec<Interval> = Vec::new();
        for window in &windows {
            let offset = window.turn as f64 * period;
            for (start, end) in nearest
                .iter()
                .filter(|index| window.segments.contains(index))
                .filter_map(|index| self.segment(*index))
            {
                let low = (start.parameter + offset).max(range.start());
                let high = (end.parameter + offset).min(range.end());
                let Some(piece) = Interval::new(low, high).filter(|piece| piece.length() > 0.0)
                else {
                    continue;
                };
                match runs.last_mut() {
                    Some(last) if last.end() == piece.start() => {
                        *last = Interval::new(last.start(), piece.end()).unwrap_or(*last);
                    }
                    _ => runs.push(piece),
                }
            }
        }
        if runs.is_empty() {
            return vec![range];
        }
        runs
    }

    fn windows(&self, range: Interval) -> (Interval, Vec<Window>) {
        let domain = self.domain();
        let turns = match self.period().filter(|period| *period > 0.0) {
            Some(period) => {
                let low = ((range.start() - domain.start()) / period).floor();
                let high = ((range.end() - domain.start()) / period).floor();
                let low = if low.is_finite() { low as i64 } else { 0 };
                let high = if high.is_finite() { high as i64 } else { 0 };
                low..=high.min(low + MAX_UNROLLED_PERIODS)
            }
            None => 0..=0,
        };
        let range = if self.closed {
            range
        } else {
            let start = domain.clamp(range.start());
            Interval::new(start, domain.clamp(range.end()).max(start)).unwrap_or(domain)
        };
        let windows = turns
            .map(|turn| {
                let offset = turn as f64 * domain.length();
                let from = self
                    .nodes
                    .partition_point(|node| node.parameter + offset < range.start())
                    .saturating_sub(1);
                let to = self
                    .nodes
                    .partition_point(|node| node.parameter + offset <= range.end())
                    .min(self.nodes.len().saturating_sub(1));
                Window {
                    turn,
                    segments: from..to.max(from),
                }
            })
            .collect();
        (range, windows)
    }

    pub(crate) fn evaluate(&self, parameter: f64) -> [Point3; 3] {
        let parameter = self.wrapped(parameter);
        match self.segment_at(parameter) {
            Some((start, end)) => hermite(start, end, parameter),
            None => [Point3::ZERO; 3],
        }
    }

    pub fn point(&self, parameter: f64) -> Point3 {
        let [point, _, _] = self.evaluate(parameter);
        point
    }

    pub fn uv_hint(&self, parameter: f64) -> [Point2; 2] {
        let parameter = self.wrapped(parameter);
        let Some((start, end)) = self.segment_at(parameter) else {
            return [Point2::ZERO; 2];
        };
        let span = end.parameter - start.parameter;
        let fraction = if span > 0.0 {
            ((parameter - start.parameter) / span).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let [a0, a1] = start.uv;
        let [b0, b1] = end.uv;
        [a0.lerp(b0, fraction), a1.lerp(b1, fraction)]
    }

    pub fn uv_at(&self, parameter: f64) -> [Point2; 2] {
        let point = self.point(parameter);
        let [first_hint, second_hint] = self.uv_hint(parameter);
        let [first, second] = &*self.surfaces;
        [
            first.project(point, Some(first_hint)),
            second.project(point, Some(second_hint)),
        ]
    }

    pub fn refined_point(&self, parameter: f64) -> Point3 {
        let [point, derivative, _] = self.evaluate(parameter);
        let [first, second] = &*self.surfaces;
        let Some(normal) = derivative.try_normalize() else {
            return point;
        };
        refine_on_plane([first, second], self.uv_at(parameter), point, normal)
            .map_or(point, |contact| contact.point)
    }

    fn pieces(&self, range: Interval) -> Vec<(IntersectionNode, IntersectionNode)> {
        let (Some(first), Some(last)) = (self.nodes.first(), self.nodes.last()) else {
            return Vec::new();
        };
        let uv_shift = if self.closed {
            let [a0, a1] = first.uv;
            let [b0, b1] = last.uv;
            [b0 - a0, b1 - a1]
        } else {
            [Point2::ZERO; 2]
        };
        let period = self.domain().length();
        let (range, windows) = self.windows(range);
        let mut pieces = Vec::new();
        for window in windows {
            let turn = window.turn as f64;
            let shift = |node: &IntersectionNode| {
                let [s0, s1] = uv_shift;
                let [u0, u1] = node.uv;
                IntersectionNode {
                    parameter: node.parameter + turn * period,
                    uv: [u0 + s0 * turn, u1 + s1 * turn],
                    ..*node
                }
            };
            for (a, b) in window.segments.filter_map(|index| self.segment(index)) {
                let (a, b) = (shift(a), shift(b));
                if b.parameter < range.start() || a.parameter > range.end() {
                    continue;
                }
                let low = a.parameter.max(range.start());
                let high = b.parameter.min(range.end());
                if high < low {
                    continue;
                }
                let cut = |parameter: f64| {
                    if parameter == a.parameter {
                        a
                    } else if parameter == b.parameter {
                        b
                    } else {
                        let [point, derivative, _] = hermite(&a, &b, parameter);
                        let fraction = (parameter - a.parameter) / (b.parameter - a.parameter);
                        let [a0, a1] = a.uv;
                        let [b0, b1] = b.uv;
                        IntersectionNode {
                            parameter,
                            point,
                            derivative,
                            uv: [a0.lerp(b0, fraction), a1.lerp(b1, fraction)],
                        }
                    }
                };
                pieces.push((cut(low), cut(high)));
            }
        }
        pieces
    }

    pub(crate) fn bounds(&self, range: Interval) -> Aabb {
        self.pieces(range)
            .iter()
            .map(|(start, end)| segment_hull(start, end))
            .reduce(Aabb::union)
            .unwrap_or_else(|| Aabb::from_point(self.point(range.start())))
    }

    pub(crate) fn seeds(&self, range: Interval) -> Vec<f64> {
        let apart = |a: f64, b: f64| (b - a).abs() > SEED_GAP;
        let mut seeds: Vec<f64> = self
            .pieces(range)
            .into_iter()
            .flat_map(|(a, b)| [a.parameter, b.parameter])
            .filter(|seed| {
                range.contains(*seed) && apart(range.start(), *seed) && apart(*seed, range.end())
            })
            .collect();
        seeds.sort_by(f64::total_cmp);
        seeds.dedup_by(|later, earlier| !apart(*earlier, *later));
        seeds.insert(0, range.start());
        seeds.push(range.end());
        seeds
    }

    pub fn trimmed(&self, range: Interval) -> Option<Self> {
        let pieces = self.pieces(range);
        let mut nodes: Vec<IntersectionNode> = Vec::with_capacity(pieces.len() + 1);
        for (a, b) in pieces {
            if nodes.is_empty() {
                nodes.push(a);
            }
            if nodes.last().is_none_or(|last| b.parameter > last.parameter) {
                nodes.push(b);
            }
        }
        let [first, second] = &*self.surfaces;
        for node in &mut nodes {
            let [first_hint, second_hint] = node.uv;
            node.uv = [
                first.project(node.point, Some(first_hint)),
                second.project(node.point, Some(second_hint)),
            ];
        }
        let full_turn = self
            .period()
            .is_some_and(|period| (range.length() - period).abs() <= 1e-12 * (1.0 + period));
        Self::new((*self.surfaces).clone(), nodes, full_turn).ok()
    }

    pub fn reversal_pivot(&self) -> f64 {
        let domain = self.domain();
        domain.start() + domain.end()
    }

    #[must_use]
    pub fn reversed(&self) -> Self {
        let pivot = self.reversal_pivot();
        let nodes: Vec<IntersectionNode> = self
            .nodes
            .iter()
            .rev()
            .map(|node| IntersectionNode {
                parameter: pivot - node.parameter,
                derivative: -node.derivative,
                ..*node
            })
            .collect();
        Self {
            surfaces: Arc::clone(&self.surfaces),
            nodes: nodes.into(),
            closed: self.closed,
            index: Arc::default(),
        }
    }

    pub(crate) fn mapped(&self, similarity: &Similarity) -> Result<Self, GeometryError> {
        let [first, second] = &*self.surfaces;
        let (first, first_map) = first.mapped(similarity)?;
        let (second, second_map) = second.mapped(similarity)?;
        let scale = similarity.scale();
        let nodes = self
            .nodes
            .iter()
            .map(|node| {
                let [first_uv, second_uv] = node.uv;
                IntersectionNode {
                    parameter: node.parameter * scale,
                    point: similarity.apply_point(node.point),
                    derivative: similarity.apply_direction(node.derivative),
                    uv: [first_map.apply(first_uv), second_map.apply(second_uv)],
                }
            })
            .collect();
        Self::new([first, second], nodes, self.closed)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Touching {
    Refuse,
    Follow,
}

fn touching_contact(surfaces: [&Surface; 2], start: [Point2; 2], point: Point3) -> Option<Contact> {
    let [first, second] = surfaces;
    let [mut first_uv, mut second_uv] = start;
    let mut current = point;
    for _ in 0..TOUCHING_ITERATIONS {
        first_uv = first.project(current, Some(first_uv));
        let on_first = first.point_at(first_uv);
        second_uv = second.project(on_first, Some(second_uv));
        let on_second = second.point_at(second_uv);
        let gap = on_first.distance(on_second);
        current = on_first.lerp(on_second, 0.5);
        if gap <= TOUCHING_GAP {
            return Some(Contact {
                uv: [
                    first.project(current, Some(first_uv)),
                    second.project(current, Some(second_uv)),
                ],
                point: current,
            });
        }
    }
    None
}

fn refine_traced(
    surfaces: [&Surface; 2],
    traced: &[Traced],
    touching: Touching,
) -> Option<Vec<Traced>> {
    let Some(first) = traced.first() else {
        return Some(Vec::new());
    };
    let mut refined = vec![*first];
    for pair in traced.windows(2) {
        let [start, end] = pair else {
            continue;
        };
        let mut pending = vec![(*start, *end, 0usize)];
        while let Some((low, high, depth)) = pending.pop() {
            interrupt::check().ok()?;
            let split = (depth < MAX_REFINE_DEPTH && refined.len() + pending.len() < MAX_NODES)
                .then(|| midpoint(surfaces, &low, &high, touching))
                .flatten();
            match split {
                Some(middle) => {
                    pending.push((middle, high, depth + 1));
                    pending.push((low, middle, depth + 1));
                }
                None => refined.push(high),
            }
        }
    }
    Some(refined)
}

fn midpoint(
    surfaces: [&Surface; 2],
    low: &Traced,
    high: &Traced,
    touching: Touching,
) -> Option<Traced> {
    let span = arc_estimate(
        low.contact.point,
        low.tangent,
        high.contact.point,
        high.tangent,
    );
    if span <= 2.0 * LINEAR_RESOLUTION {
        return None;
    }
    let as_node = |item: &Traced, parameter: f64| IntersectionNode {
        parameter,
        point: item.contact.point,
        derivative: item.tangent,
        uv: item.contact.uv,
    };
    let (start, end) = (as_node(low, 0.0), as_node(high, span));
    let [guess, derivative, _] = hermite(&start, &end, 0.5 * span);
    let turn = angle_between(low.tangent, high.tangent);
    let [a0, a1] = low.contact.uv;
    let [b0, b1] = high.contact.uv;
    let [first, second] = surfaces;
    let hint = [
        first.project(guess, Some(a0.lerp(b0, 0.5))),
        second.project(guess, Some(a1.lerp(b1, 0.5))),
    ];
    let [first_hint, second_hint] = hint;
    let normal = derivative
        .try_normalize()
        .or_else(|| (high.contact.point - low.contact.point).try_normalize())?;
    let contact = match touching {
        Touching::Refuse => refine_on_plane(surfaces, hint, guess, normal)?,
        Touching::Follow => refine_on_plane(surfaces, hint, guess, normal)
            .or_else(|| touching_contact(surfaces, hint, guess))?,
    };
    let deviation = contact.point.distance(guess);
    if deviation <= INTERSECTION_TOLERANCE && turn <= MAX_SEGMENT_TURN {
        return None;
    }
    let between = (contact.point - low.contact.point).length() > LINEAR_RESOLUTION
        && (high.contact.point - contact.point).length() > LINEAR_RESOLUTION;
    if !between {
        return None;
    }
    let tangent = contact_direction(surfaces, &contact)
        .filter(|(_, sine)| *sine > 1e-9)
        .map_or(normal, |(direction, _)| direction);
    let tangent = if tangent.dot(normal) < 0.0 {
        -tangent
    } else {
        tangent
    };
    let [first_uv, second_uv] = contact.uv;
    let continuous = [
        continuous_uv(first, first_uv, first_hint),
        continuous_uv(second, second_uv, second_hint),
    ];
    Some(Traced {
        contact: Contact {
            uv: continuous,
            ..contact
        },
        tangent,
    })
}

fn continuous_uv(surface: &Surface, uv: Point2, hint: Point2) -> Point2 {
    let u = match surface.u_period() {
        Some(period) => periodic_near(uv.x, period, Some(hint.x)),
        None => uv.x,
    };
    let v = match surface.v_period() {
        Some(period) => periodic_near(uv.y, period, Some(hint.y)),
        None => uv.y,
    };
    Point2::new(u, v)
}
