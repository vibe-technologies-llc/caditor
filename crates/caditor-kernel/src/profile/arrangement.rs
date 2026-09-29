use std::{
    collections::{BTreeMap, BTreeSet},
    f64::consts::{PI, TAU},
};

use caditor_geometry::{Aabb, Aabb2, Point2, Point3, Vector2};

use crate::{
    box_tree::BoxTree,
    curve2::{Curve2, Line2},
    interrupt,
    interval::Interval,
    profile::{
        PieceBound, PieceId, ProfileCurve, ProfileError,
        geometry::{area_under, winding},
        intersect::{self, Scale},
        source::Source,
    },
    tolerance::LINEAR_RESOLUTION,
};

const RELATIVE_TOLERANCE: f64 = 1e-7;
const MERGE_TOLERANCES: f64 = 64.0;
const SAME_PATH_TOLERANCES: f64 = 8.0;
const SAME_PATH_FRACTIONS: [f64; 3] = [0.25, 0.5, 0.75];
const ANGLE_TIE: f64 = 1e-2;
const PROBE_FRACTION: f64 = 0.05;
const TANGENT_NUDGE: f64 = 1e-6;

type Found<T> = Result<T, ProfileError>;

fn flat(bounds: &Aabb2) -> Aabb {
    let lift = |point: Point2| Point3::new(point.x, point.y, 0.0);
    Aabb::from_point(lift(bounds.min())).including(lift(bounds.max()))
}

fn lookup<T: Copy>(items: &[T], index: usize) -> Found<T> {
    items
        .get(index)
        .copied()
        .ok_or_else(ProfileError::unresolved)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum EventKind {
    Start,
    Cut,
    Synthetic,
    End,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Event {
    source: usize,
    parameter: f64,
    point: Point2,
    kind: EventKind,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct GraphPiece {
    pub id: PieceId,
    pub curve: Curve2,
    pub range: Interval,
    pub start: usize,
    pub end: usize,
}

impl GraphPiece {
    pub fn origin(&self, forward: bool) -> usize {
        if forward { self.start } else { self.end }
    }

    pub fn signed_area(&self, forward: bool) -> f64 {
        let area = area_under(&self.curve, self.range);
        if forward { area } else { -area }
    }

    fn length(&self) -> f64 {
        self.curve.length(self.range)
    }

    fn point_at(&self, fraction: f64) -> Point2 {
        self.curve.point(self.range.at(fraction))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct GraphFace {
    pub cycle: Vec<usize>,
    pub area: f64,
    pub depth: usize,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub(super) struct Arrangement {
    pub tolerance: f64,
    pub vertices: Vec<Point2>,
    pub pieces: Vec<GraphPiece>,
    pub order: Vec<Vec<usize>>,
    pub position: Vec<usize>,
    pub faces: Vec<GraphFace>,
    pub sides: Vec<[Option<usize>; 2]>,
    pub face_half_edges: Vec<Vec<usize>>,
}

pub(super) fn piece_of(half_edge: usize) -> (usize, bool) {
    (half_edge / 2, half_edge.is_multiple_of(2))
}

impl Arrangement {
    pub fn new(curves: &[ProfileCurve]) -> Found<Self> {
        let (sources, scale) = sources(curves)?;
        if sources.is_empty() {
            return Ok(Self::default());
        }
        let events = events(&sources, scale)?;
        let (vertices, vertex_of) = cluster(&events, &sources, scale.tolerance)?;
        let raw = split(&sources, &events, &vertex_of, &vertices, scale.tolerance)?;
        let merged = merge_overlaps(raw, &sources, scale.tolerance)?;
        let pieces = settle(merged, vertices.len())?;
        let mut arrangement = Self {
            tolerance: scale.tolerance,
            vertices,
            pieces,
            ..Self::default()
        };
        arrangement.connect()?;
        let cycles = arrangement.cycles(|_| true)?;
        arrangement.find_faces(&cycles)?;
        Ok(arrangement)
    }

    pub fn tolerance(&self) -> f64 {
        self.tolerance
    }

    pub fn face_count(&self) -> usize {
        self.faces.len()
    }

    pub fn origin(&self, half_edge: usize) -> Found<usize> {
        let (piece, forward) = piece_of(half_edge);
        Ok(self
            .pieces
            .get(piece)
            .ok_or_else(ProfileError::unresolved)?
            .origin(forward))
    }

    pub fn destination(&self, half_edge: usize) -> Found<usize> {
        self.origin(half_edge ^ 1)
    }

    pub fn half_edge_area(&self, half_edge: usize) -> f64 {
        let (piece, forward) = piece_of(half_edge);
        self.pieces
            .get(piece)
            .map_or(0.0, |piece| piece.signed_area(forward))
    }

    pub fn contains(&self, cycle: &[usize], point: Point2) -> bool {
        let segments = cycle.iter().filter_map(|half_edge| {
            let (piece, forward) = piece_of(*half_edge);
            self.pieces
                .get(piece)
                .map(|piece| (&piece.curve, piece.range, !forward))
        });
        winding(segments, point) != 0
    }

    pub fn next_where(&self, half_edge: usize, kept: impl Fn(usize) -> bool) -> Found<usize> {
        let vertex = self.destination(half_edge)?;
        let list = self
            .order
            .get(vertex)
            .ok_or_else(ProfileError::unresolved)?;
        let at = lookup(&self.position, half_edge ^ 1)?;
        let count = list.len();
        for step in 1..=count {
            let candidate = lookup(list, (at + count * 2 - step) % count)?;
            if kept(candidate) {
                return Ok(candidate);
            }
        }
        Err(ProfileError::unresolved())
    }

    pub fn cycles(&self, kept: impl Fn(usize) -> bool) -> Found<Vec<Vec<usize>>> {
        self.cycles_from(0..self.pieces.len() * 2, kept)
    }

    pub fn cycles_from(
        &self,
        starts: impl IntoIterator<Item = usize>,
        kept: impl Fn(usize) -> bool,
    ) -> Found<Vec<Vec<usize>>> {
        let count = self.pieces.len() * 2;
        let mut visited = BTreeSet::new();
        let mut cycles = Vec::new();
        for start in starts {
            if start >= count || !kept(start) || visited.contains(&start) {
                continue;
            }
            let mut cycle = Vec::new();
            let mut current = start;
            while visited.insert(current) {
                cycle.push(current);
                current = self.next_where(current, &kept)?;
                if cycle.len() > count {
                    return Err(ProfileError::unresolved());
                }
            }
            if current != start {
                return Err(ProfileError::unresolved());
            }
            cycles.push(cycle);
        }
        Ok(cycles)
    }

    pub fn face_of(&self, half_edge: usize) -> Option<usize> {
        let (piece, forward) = piece_of(half_edge);
        let [left, right] = *self.sides.get(piece)?;
        if forward { left } else { right }
    }

    pub fn half_edges_of(&self, face: usize) -> &[usize] {
        self.face_half_edges.get(face).map_or(&[], Vec::as_slice)
    }

    fn connect(&mut self) -> Found<()> {
        let (order, position) = angular_order_positions(&self.pieces, self.vertices.len())?;
        self.order = order;
        self.position = position;
        Ok(())
    }

    fn cycle_bounds(&self, cycle: &[usize]) -> Option<Aabb2> {
        cycle
            .iter()
            .filter_map(|half_edge| self.pieces.get(piece_of(*half_edge).0))
            .map(|piece| piece.curve.bounding_box(piece.range))
            .reduce(Aabb2::union)
    }

    fn cycle_area(&self, cycle: &[usize]) -> f64 {
        cycle
            .iter()
            .map(|half_edge| self.half_edge_area(*half_edge))
            .sum()
    }

    fn find_faces(&mut self, cycles: &[Vec<usize>]) -> Found<()> {
        let threshold = self.tolerance * self.tolerance;
        let components = components(&self.pieces, self.vertices.len());
        let component_of = |cycle: &[usize]| -> Found<usize> {
            let first = *cycle.first().ok_or_else(ProfileError::unresolved)?;
            lookup(&components, self.origin(first)?)
        };
        let mut faces: Vec<(usize, f64, usize)> = Vec::new();
        let mut outer: BTreeMap<usize, usize> = BTreeMap::new();
        let mut face_of_cycle: Vec<Option<usize>> = vec![None; cycles.len()];
        for (index, cycle) in cycles.iter().enumerate() {
            let area = self.cycle_area(cycle);
            let component = component_of(cycle)?;
            if area > threshold {
                if let Some(slot) = face_of_cycle.get_mut(index) {
                    *slot = Some(faces.len());
                }
                faces.push((index, area, component));
            } else {
                outer.entry(component).or_insert(index);
            }
        }
        let face_bounds: Vec<Aabb> = faces
            .iter()
            .map(|(cycle, _, _)| {
                cycles
                    .get(*cycle)
                    .and_then(|cycle| self.cycle_bounds(cycle))
                    .map_or_else(|| Aabb::from_point(Point3::ZERO), |bounds| flat(&bounds))
            })
            .collect();
        let tree = BoxTree::new(face_bounds);
        let mut parent: BTreeMap<usize, usize> = BTreeMap::new();
        for (component, cycle) in &outer {
            let cycle = cycles.get(*cycle).ok_or_else(ProfileError::unresolved)?;
            let first = *cycle.first().ok_or_else(ProfileError::unresolved)?;
            let probe = lookup(&self.vertices, self.origin(first)?)?;
            let container = tree
                .overlapping(&flat(&Aabb2::from_point(probe)), self.tolerance)
                .into_iter()
                .filter_map(|face| Some((face, faces.get(face)?)))
                .filter(|(_, (_, _, owner))| owner != component)
                .filter(|(_, (face_cycle, _, _))| {
                    cycles
                        .get(*face_cycle)
                        .is_some_and(|face_cycle| self.contains(face_cycle, probe))
                })
                .min_by(|a, b| a.1.1.total_cmp(&b.1.1))
                .map(|(face, _)| face);
            if let Some(face) = container {
                parent.insert(*component, face);
            }
        }
        let mut cycle_of = vec![usize::MAX; self.pieces.len() * 2];
        for (index, cycle) in cycles.iter().enumerate() {
            for half_edge in cycle {
                if let Some(slot) = cycle_of.get_mut(*half_edge) {
                    *slot = index;
                }
            }
        }
        let side_of = |half_edge: usize| -> Found<Option<usize>> {
            let cycle = lookup(&cycle_of, half_edge)?;
            match face_of_cycle.get(cycle).copied().flatten() {
                Some(face) => Ok(Some(face)),
                None => {
                    let cycle = cycles.get(cycle).ok_or_else(ProfileError::unresolved)?;
                    Ok(parent.get(&component_of(cycle)?).copied())
                }
            }
        };
        self.sides = (0..self.pieces.len())
            .map(|piece| Ok([side_of(piece * 2)?, side_of(piece * 2 + 1)?]))
            .collect::<Found<_>>()?;
        let enclosing = |face: usize| -> Option<usize> {
            let (cycle, _, _) = faces.get(face)?;
            let mut beyond = cycles
                .get(*cycle)?
                .iter()
                .map(|half_edge| self.face_of(half_edge ^ 1));
            let first = beyond.next()??;
            (first != face && beyond.all(|other| other == Some(first))).then_some(first)
        };
        let up: Vec<Option<usize>> = faces
            .iter()
            .enumerate()
            .map(|(face, (_, _, component))| {
                enclosing(face).or_else(|| parent.get(component).copied())
            })
            .collect();
        let depths = nesting_depths(&up);
        let mut face_half_edges = vec![Vec::new(); faces.len()];
        for half_edge in 0..self.pieces.len() * 2 {
            if let Some(list) = self
                .face_of(half_edge)
                .and_then(|face| face_half_edges.get_mut(face))
            {
                list.push(half_edge);
            }
        }
        self.face_half_edges = face_half_edges;
        self.faces = faces
            .iter()
            .zip(depths)
            .map(|((cycle, area, _), depth)| {
                Ok(GraphFace {
                    cycle: cycles
                        .get(*cycle)
                        .ok_or_else(ProfileError::unresolved)?
                        .clone(),
                    area: *area,
                    depth,
                })
            })
            .collect::<Found<_>>()?;
        Ok(())
    }
}

fn nesting_depths(up: &[Option<usize>]) -> Vec<usize> {
    let mut depths: Vec<Option<usize>> = vec![None; up.len()];
    for start in 0..up.len() {
        let mut chain = Vec::new();
        let mut current = Some(start);
        let mut depth = 0;
        while let Some(face) = current {
            if let Some(known) = depths.get(face).copied().flatten() {
                depth = known + 1;
                break;
            }
            if chain.len() > up.len() {
                break;
            }
            chain.push(face);
            current = up.get(face).copied().flatten();
        }
        for face in chain.into_iter().rev() {
            if let Some(slot) = depths.get_mut(face) {
                *slot = Some(depth);
            }
            depth += 1;
        }
    }
    depths.into_iter().map(Option::unwrap_or_default).collect()
}

fn sources(curves: &[ProfileCurve]) -> Found<(Vec<Source>, Scale)> {
    let mut sorted: Vec<&ProfileCurve> = curves.iter().collect();
    sorted.sort_by_key(|curve| curve.entity);
    for pair in sorted.windows(2) {
        if let [first, second] = pair
            && first.entity == second.entity
        {
            return Err(ProfileError::DuplicateEntity {
                entity: first.entity,
            });
        }
    }
    let sources = sorted
        .into_iter()
        .map(Source::from_curve)
        .collect::<Found<Vec<_>>>()?;
    let size = sources
        .iter()
        .map(Source::bounds)
        .reduce(Aabb2::union)
        .map_or(0.0, |bounds| bounds.size().length());
    let tolerance = (size * RELATIVE_TOLERANCE).max(LINEAR_RESOLUTION);
    for source in &sources {
        let length = source.curve.length(source.range);
        if length.is_nan() || length <= tolerance {
            return Err(ProfileError::Degenerate {
                entity: source.entity,
            });
        }
    }
    Ok((
        sources,
        Scale {
            tolerance,
            size: size.max(tolerance),
        },
    ))
}

fn events(sources: &[Source], scale: Scale) -> Found<Vec<Event>> {
    let tolerance = scale.tolerance;
    let segments: Vec<Vec<Interval>> = sources.iter().map(Source::monotone_segments).collect();
    let bounds: Vec<Aabb2> = sources
        .iter()
        .map(|source| source.bounds().expanded(tolerance))
        .collect();
    let mut events = Vec::new();
    for (index, source) in sources.iter().enumerate() {
        if !source.closed {
            for (parameter, kind) in [
                (source.range.start(), EventKind::Start),
                (source.range.end(), EventKind::End),
            ] {
                events.push(Event {
                    source: index,
                    parameter,
                    point: source.point(parameter),
                    kind,
                });
            }
        }
    }
    let cut = |source: usize, parameter: f64, point: Point2| Event {
        source,
        parameter,
        point,
        kind: EventKind::Cut,
    };
    let tree = BoxTree::new(bounds.iter().map(flat));
    for (first, first_source) in sources.iter().enumerate() {
        interrupt::check()?;
        let Some(first_bounds) = bounds.get(first) else {
            continue;
        };
        let later = tree
            .overlapping(&flat(first_bounds), 0.0)
            .into_iter()
            .filter(|second| *second > first);
        for second in later {
            let Some(second_source) = sources.get(second) else {
                continue;
            };
            let hits = intersect::between(
                first_source,
                segments.get(first).map_or(&[], Vec::as_slice),
                second_source,
                segments.get(second).map_or(&[], Vec::as_slice),
                scale,
            )
            .map_err(|failure| match failure {
                intersect::Unresolved::Overlapping => ProfileError::Overlap {
                    first: first_source.entity,
                    second: second_source.entity,
                },
                intersect::Unresolved::TooIntricate => ProfileError::TooIntricate {
                    entities: vec![first_source.entity, second_source.entity],
                },
                intersect::Unresolved::Cancelled(interrupted) => interrupted.into(),
            })?;
            for hit in hits {
                events.push(cut(first, hit.first, hit.point));
                events.push(cut(second, hit.second, hit.point));
            }
        }
        if first_source.is_spline() {
            let hits = intersect::within(
                first_source,
                segments.get(first).map_or(&[], Vec::as_slice),
                scale,
            )
            .map_err(|failure| match failure {
                intersect::Unresolved::Overlapping => ProfileError::SelfOverlap {
                    entity: first_source.entity,
                },
                intersect::Unresolved::TooIntricate => ProfileError::TooIntricate {
                    entities: vec![first_source.entity],
                },
                intersect::Unresolved::Cancelled(interrupted) => interrupted.into(),
            })?;
            for hit in hits {
                events.push(cut(first, hit.first, hit.point));
                events.push(cut(first, hit.second, hit.point));
            }
        }
    }
    for (owner, source) in sources.iter().enumerate() {
        if source.closed {
            continue;
        }
        for point in [
            source.point(source.range.start()),
            source.point(source.range.end()),
        ] {
            let near = tree.overlapping(&flat(&Aabb2::from_point(point)), 0.0);
            for (other, target) in near
                .into_iter()
                .filter_map(|other| Some((other, sources.get(other)?)))
            {
                if other == owner {
                    continue;
                }
                let parameter = target.curve.closest_parameter(point, target.range);
                if target.point(parameter).distance(point) <= tolerance {
                    events.push(cut(other, parameter, point));
                }
            }
        }
    }
    let mut has_events = vec![false; sources.len()];
    for event in &events {
        if let Some(flag) = has_events.get_mut(event.source) {
            *flag = true;
        }
    }
    for ((index, source), has_events) in sources.iter().enumerate().zip(has_events) {
        if source.closed && !has_events {
            events.push(Event {
                source: index,
                parameter: source.range.start(),
                point: source.point(source.range.start()),
                kind: EventKind::Synthetic,
            });
        }
    }
    for event in &mut events {
        if let Some(source) = sources.get(event.source) {
            event.parameter = source.wrapped(event.parameter);
        }
    }
    Ok(events)
}

struct UnionFind(Vec<usize>);

impl UnionFind {
    fn new(count: usize) -> Self {
        Self((0..count).collect())
    }

    fn find(&mut self, mut item: usize) -> usize {
        while let Some(parent) = self.0.get(item).copied() {
            if parent == item {
                break;
            }
            let grandparent = self.0.get(parent).copied().unwrap_or(parent);
            if let Some(slot) = self.0.get_mut(item) {
                *slot = grandparent;
            }
            item = parent;
        }
        item
    }

    fn union(&mut self, a: usize, b: usize) {
        let (a, b) = (self.find(a), self.find(b));
        let (low, high) = if a <= b { (a, b) } else { (b, a) };
        if let Some(slot) = self.0.get_mut(high) {
            *slot = low;
        }
    }
}

fn cluster(
    events: &[Event],
    sources: &[Source],
    tolerance: f64,
) -> Found<(Vec<Point2>, Vec<usize>)> {
    let mut sets = UnionFind::new(events.len());
    let cell_of = |point: Point2| {
        let index = |value: f64| (value / tolerance).floor() as i64;
        (index(point.x), index(point.y))
    };
    let mut cells: BTreeMap<(i64, i64), Vec<usize>> = BTreeMap::new();
    for (index, event) in events.iter().enumerate() {
        let (x, y) = cell_of(event.point);
        for column in [x.saturating_sub(1), x, x.saturating_add(1)] {
            for (_, members) in
                cells.range((column, y.saturating_sub(1))..=(column, y.saturating_add(1)))
            {
                for other in members {
                    if lookup(events, *other)?.point.distance(event.point) <= tolerance {
                        sets.union(index, *other);
                    }
                }
            }
        }
        cells.entry((x, y)).or_default().push(index);
    }
    let mut dense: BTreeMap<usize, usize> = BTreeMap::new();
    let mut vertex_of = Vec::with_capacity(events.len());
    for index in 0..events.len() {
        let root = sets.find(index);
        let next = dense.len();
        vertex_of.push(*dense.entry(root).or_insert(next));
    }
    let mut members: Vec<Vec<Event>> = vec![Vec::new(); dense.len()];
    for (event, vertex) in events.iter().zip(&vertex_of) {
        if let Some(list) = members.get_mut(*vertex) {
            list.push(*event);
        }
    }
    let vertices = members
        .iter()
        .map(|list| {
            let curved: Vec<Point2> = list
                .iter()
                .filter(|event| {
                    sources
                        .get(event.source)
                        .is_some_and(|source| !source.is_line())
                })
                .map(|event| event.point)
                .collect();
            let chosen: Vec<Point2> = if curved.is_empty() {
                list.iter().map(|event| event.point).collect()
            } else {
                curved
            };
            chosen.iter().fold(Point2::ZERO, |sum, point| sum + *point) / chosen.len().max(1) as f64
        })
        .collect();
    Ok((vertices, vertex_of))
}

#[derive(Debug, Clone, PartialEq)]
struct RawPiece {
    source: usize,
    piece: GraphPiece,
}

fn split(
    sources: &[Source],
    events: &[Event],
    vertex_of: &[usize],
    vertices: &[Point2],
    tolerance: f64,
) -> Found<Vec<RawPiece>> {
    let merge_length = MERGE_TOLERANCES * tolerance;
    let mut at_vertex: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (index, vertex) in vertex_of.iter().enumerate() {
        at_vertex.entry(*vertex).or_default().push(index);
    }
    let mut by_source: Vec<Vec<usize>> = vec![Vec::new(); sources.len()];
    for (index, event) in events.iter().enumerate() {
        if let Some(list) = by_source.get_mut(event.source) {
            list.push(index);
        }
    }
    let mut pieces = Vec::new();
    for ((index, source), mut own) in sources.iter().enumerate().zip(by_source) {
        interrupt::check()?;
        own.sort_by(|a, b| {
            let (Some(a), Some(b)) = (events.get(*a), events.get(*b)) else {
                return std::cmp::Ordering::Equal;
            };
            a.parameter
                .total_cmp(&b.parameter)
                .then(a.kind.cmp(&b.kind))
        });
        let mut kept: Vec<usize> = Vec::new();
        for event in own {
            let current = lookup(events, event)?;
            let vertex = lookup(vertex_of, event)?;
            let merge = match kept.last() {
                Some(last) => {
                    let previous = lookup(events, *last)?;
                    lookup(vertex_of, *last)? == vertex
                        && !(previous.kind == EventKind::Start && current.kind == EventKind::End)
                        && source.length_between(previous.parameter, current.parameter)
                            <= merge_length
                }
                None => false,
            };
            if !merge {
                kept.push(event);
            } else if current.kind != EventKind::Cut
                && let Some(last) = kept.last_mut()
            {
                *last = event;
            }
        }
        if let Some(period) = source.period()
            && kept.len() > 1
            && let (Some(first), Some(last)) = (kept.first(), kept.last())
        {
            let (first_event, last_event) = (lookup(events, *first)?, lookup(events, *last)?);
            let same_vertex = lookup(vertex_of, *first)? == lookup(vertex_of, *last)?;
            let gap = source.length_between(last_event.parameter, first_event.parameter + period);
            if same_vertex && gap <= merge_length {
                kept.pop();
            }
        }
        let bounds = bounds_of(index, sources, events, vertex_of, &at_vertex, &kept)?;
        let mut pairs: Vec<(usize, usize, f64, f64)> = Vec::new();
        for (position, event) in kept.iter().enumerate() {
            let start = lookup(events, *event)?;
            match kept.get(position + 1) {
                Some(next) => {
                    pairs.push((
                        position,
                        position + 1,
                        start.parameter,
                        lookup(events, *next)?.parameter,
                    ));
                }
                None => {
                    if let Some(period) = source.period() {
                        let first = kept.first().ok_or_else(ProfileError::unresolved)?;
                        let end = lookup(events, *first)?.parameter + period;
                        pairs.push((position, 0, start.parameter, end));
                    }
                }
            }
        }
        for (from, to, low, high) in pairs {
            let (Some(from_event), Some(to_event)) = (kept.get(from), kept.get(to)) else {
                continue;
            };
            let (start, end) = (
                lookup(vertex_of, *from_event)?,
                lookup(vertex_of, *to_event)?,
            );
            let Some(range) = Interval::new(low, high) else {
                continue;
            };
            if range.length() <= 0.0 || (start == end && source.curve.length(range) <= merge_length)
            {
                continue;
            }
            let start_bound = bounds
                .get(from)
                .cloned()
                .ok_or_else(ProfileError::unresolved)?;
            let end_bound = bounds
                .get(to)
                .cloned()
                .ok_or_else(ProfileError::unresolved)?;
            let id = PieceId::new(
                source.entity,
                match start_bound {
                    PieceBound::End => PieceBound::Start,
                    other => other,
                },
                match end_bound {
                    PieceBound::Start => PieceBound::End,
                    other => other,
                },
            );
            let (curve, range) = if source.is_line() {
                let (from_point, to_point) = (lookup(vertices, start)?, lookup(vertices, end)?);
                let Ok(line) = Line2::through(from_point, to_point) else {
                    continue;
                };
                let Some(range) = Interval::new(0.0, from_point.distance(to_point)) else {
                    continue;
                };
                (Curve2::from(line), range)
            } else {
                (source.curve.clone(), range)
            };
            pieces.push(RawPiece {
                source: index,
                piece: GraphPiece {
                    id,
                    curve,
                    range,
                    start,
                    end,
                },
            });
        }
    }
    Ok(pieces)
}

fn bounds_of(
    index: usize,
    sources: &[Source],
    events: &[Event],
    vertex_of: &[usize],
    at_vertex: &BTreeMap<usize, Vec<usize>>,
    kept: &[usize],
) -> Found<Vec<PieceBound>> {
    let entity = |source: usize| sources.get(source).map(|source| source.entity);
    let own_entity = entity(index).ok_or_else(ProfileError::unresolved)?;
    let mut cutters: Vec<Option<Vec<u64>>> = Vec::with_capacity(kept.len());
    for event in kept {
        let current = lookup(events, *event)?;
        if current.kind != EventKind::Cut {
            cutters.push(None);
            continue;
        }
        let vertex = lookup(vertex_of, *event)?;
        let mut set: BTreeSet<u64> = at_vertex
            .get(&vertex)
            .into_iter()
            .flatten()
            .filter_map(|other| events.get(*other))
            .filter(|other| other.source != index)
            .filter_map(|other| entity(other.source))
            .collect();
        let revisited = kept
            .iter()
            .any(|other| other != event && vertex_of.get(*other).copied() == Some(vertex));
        if revisited {
            set.insert(own_entity);
        }
        cutters.push(Some(set.into_iter().collect()));
    }
    let mut seen: BTreeMap<Vec<u64>, u32> = BTreeMap::new();
    Ok(kept
        .iter()
        .zip(cutters)
        .map(|(event, cutters)| match cutters {
            Some(entities) => {
                let counter = seen.entry(entities.clone()).or_insert(0);
                let occurrence = *counter;
                *counter += 1;
                PieceBound::Cut {
                    entities,
                    occurrence,
                }
            }
            None => match events.get(*event).map(|event| event.kind) {
                Some(EventKind::End) => PieceBound::End,
                _ => PieceBound::Start,
            },
        })
        .collect())
}

fn same_path(first: &GraphPiece, second: &GraphPiece, tolerance: f64) -> bool {
    let near = |from: &GraphPiece, to: &GraphPiece| {
        SAME_PATH_FRACTIONS.iter().all(|fraction| {
            let point = from.point_at(*fraction);
            let parameter = to.curve.closest_parameter(point, to.range);
            to.curve.point(parameter).distance(point) <= SAME_PATH_TOLERANCES * tolerance
        })
    };
    near(first, second) && near(second, first)
}

fn merge_overlaps(
    pieces: Vec<RawPiece>,
    sources: &[Source],
    tolerance: f64,
) -> Found<Vec<GraphPiece>> {
    let mut groups: BTreeMap<(usize, usize), Vec<usize>> = BTreeMap::new();
    for (index, raw) in pieces.iter().enumerate() {
        let (a, b) = (raw.piece.start, raw.piece.end);
        groups.entry((a.min(b), a.max(b))).or_default().push(index);
    }
    let mut dropped = vec![false; pieces.len()];
    for members in groups.values() {
        for (rank, first) in members.iter().enumerate() {
            for second in members.iter().skip(rank + 1) {
                if dropped.get(*first).copied().unwrap_or(true)
                    || dropped.get(*second).copied().unwrap_or(true)
                {
                    continue;
                }
                let (Some(a), Some(b)) = (pieces.get(*first), pieces.get(*second)) else {
                    continue;
                };
                if !same_path(&a.piece, &b.piece, tolerance) {
                    continue;
                }
                if a.source == b.source {
                    let entity = sources.get(a.source).map_or(0, |source| source.entity);
                    return Err(ProfileError::SelfOverlap { entity });
                }
                let loser = if a.piece.id.entity() <= b.piece.id.entity() {
                    *second
                } else {
                    *first
                };
                if let Some(flag) = dropped.get_mut(loser) {
                    *flag = true;
                }
            }
        }
    }
    Ok(pieces
        .into_iter()
        .zip(dropped)
        .filter(|(_, dropped)| !dropped)
        .map(|(raw, _)| raw.piece)
        .collect())
}

fn settle(mut pieces: Vec<GraphPiece>, vertex_count: usize) -> Found<Vec<GraphPiece>> {
    let limit = pieces.len() + 1;
    for _ in 0..limit {
        interrupt::check()?;
        pieces = pruned(pieces, vertex_count);
        let probe = Arrangement {
            vertices: Vec::new(),
            pieces: pieces.clone(),
            ..Arrangement::default()
        };
        let (order, position) = angular_order_positions(&probe.pieces, vertex_count)?;
        let probe = Arrangement {
            order,
            position,
            ..probe
        };
        let cycles = probe.cycles(|_| true)?;
        let mut cycle_of = vec![usize::MAX; pieces.len() * 2];
        for (index, cycle) in cycles.iter().enumerate() {
            for half_edge in cycle {
                if let Some(slot) = cycle_of.get_mut(*half_edge) {
                    *slot = index;
                }
            }
        }
        let bridges: BTreeSet<usize> = (0..pieces.len())
            .filter(|piece| cycle_of.get(piece * 2) == cycle_of.get(piece * 2 + 1))
            .collect();
        if bridges.is_empty() {
            return Ok(pieces);
        }
        pieces = pieces
            .into_iter()
            .enumerate()
            .filter(|(index, _)| !bridges.contains(index))
            .map(|(_, piece)| piece)
            .collect();
    }
    Err(ProfileError::unresolved())
}

fn pruned(pieces: Vec<GraphPiece>, vertex_count: usize) -> Vec<GraphPiece> {
    let mut degree = vec![0usize; vertex_count];
    let mut incident: Vec<Vec<usize>> = vec![Vec::new(); vertex_count];
    for (index, piece) in pieces.iter().enumerate() {
        for vertex in [piece.start, piece.end] {
            if let (Some(count), Some(list)) = (degree.get_mut(vertex), incident.get_mut(vertex)) {
                *count += 1;
                list.push(index);
            }
        }
    }
    let mut removed = vec![false; pieces.len()];
    let mut pending: Vec<usize> = (0..vertex_count)
        .filter(|vertex| degree.get(*vertex).is_some_and(|count| *count <= 1))
        .collect();
    while let Some(vertex) = pending.pop() {
        let around = incident.get(vertex).cloned().unwrap_or_default();
        for index in around {
            let Some(flag) = removed.get_mut(index) else {
                continue;
            };
            if *flag {
                continue;
            }
            *flag = true;
            let Some(piece) = pieces.get(index) else {
                continue;
            };
            for end in [piece.start, piece.end] {
                if let Some(count) = degree.get_mut(end) {
                    *count = count.saturating_sub(1);
                    if *count == 1 {
                        pending.push(end);
                    }
                }
            }
        }
    }
    pieces
        .into_iter()
        .zip(removed)
        .filter(|(_, removed)| !removed)
        .map(|(piece, _)| piece)
        .collect()
}

fn components(pieces: &[GraphPiece], vertex_count: usize) -> Vec<usize> {
    let mut sets = UnionFind::new(vertex_count);
    for piece in pieces {
        sets.union(piece.start, piece.end);
    }
    (0..vertex_count).map(|vertex| sets.find(vertex)).collect()
}

struct Leaving {
    half_edge: usize,
    angle: f64,
    start: Point2,
    probe_speed: f64,
    length: f64,
}

fn leaving(piece: &GraphPiece, half_edge: usize, forward: bool) -> Leaving {
    let parameter = if forward {
        piece.range.start()
    } else {
        piece.range.end()
    };
    let sign = if forward { 1.0 } else { -1.0 };
    let derivatives = piece.curve.evaluate(parameter);
    let mut tangent: Vector2 = derivatives.first * sign;
    if tangent.length().is_nan() || tangent.length() <= 0.0 {
        let nudged = parameter + sign * TANGENT_NUDGE * piece.range.length();
        tangent = piece.curve.point(nudged) - derivatives.point;
    }
    Leaving {
        half_edge,
        angle: tangent.y.atan2(tangent.x).rem_euclid(TAU),
        start: derivatives.point,
        probe_speed: derivatives.first.length(),
        length: piece.length(),
    }
}

fn probe_angle(
    piece: &GraphPiece,
    leaving: &Leaving,
    forward: bool,
    distance: f64,
    base: f64,
) -> f64 {
    let step = if leaving.probe_speed > 0.0 {
        distance / leaving.probe_speed
    } else {
        PROBE_FRACTION * piece.range.length()
    };
    let parameter = if forward {
        piece.range.clamp(piece.range.start() + step)
    } else {
        piece.range.clamp(piece.range.end() - step)
    };
    let offset = piece.curve.point(parameter) - leaving.start;
    let angle = offset.y.atan2(offset.x);
    (angle - base + PI).rem_euclid(TAU) - PI
}

fn angular_order_positions(
    pieces: &[GraphPiece],
    vertex_count: usize,
) -> Found<(Vec<Vec<usize>>, Vec<usize>)> {
    let mut outgoing: Vec<Vec<Leaving>> = (0..vertex_count).map(|_| Vec::new()).collect();
    for (index, piece) in pieces.iter().enumerate() {
        for forward in [true, false] {
            let half_edge = index * 2 + usize::from(!forward);
            let list = outgoing
                .get_mut(piece.origin(forward))
                .ok_or_else(ProfileError::unresolved)?;
            list.push(leaving(piece, half_edge, forward));
        }
    }
    let mut order = Vec::with_capacity(vertex_count);
    let mut position = vec![0usize; pieces.len() * 2];
    for mut list in outgoing {
        list.sort_by(|a, b| {
            a.angle
                .total_cmp(&b.angle)
                .then(a.half_edge.cmp(&b.half_edge))
        });
        let count = list.len();
        let widest = (0..count)
            .max_by(|a, b| {
                let gap = |index: usize| {
                    let here = list.get(index).map_or(0.0, |entry| entry.angle);
                    let next = list
                        .get((index + 1) % count.max(1))
                        .map_or(0.0, |entry| entry.angle);
                    (next - here).rem_euclid(TAU)
                };
                gap(*a).total_cmp(&gap(*b))
            })
            .unwrap_or(0);
        list.rotate_left((widest + 1) % count.max(1));
        let mut unwrapped: Vec<(Leaving, f64)> = Vec::with_capacity(count);
        let mut base = None;
        for entry in list {
            let first = *base.get_or_insert(entry.angle);
            let angle = first + (entry.angle - first).rem_euclid(TAU);
            unwrapped.push((entry, angle));
        }
        let mut keyed: Vec<(usize, f64, f64)> = Vec::with_capacity(count);
        let mut run_start = 0;
        while run_start < unwrapped.len() {
            let mut run_end = run_start + 1;
            while let (Some(previous), Some(next)) =
                (unwrapped.get(run_end - 1), unwrapped.get(run_end))
            {
                if next.1 - previous.1 > ANGLE_TIE {
                    break;
                }
                run_end += 1;
            }
            let run = unwrapped.get(run_start..run_end).unwrap_or_default();
            let base_angle = run.first().map_or(0.0, |entry| entry.1);
            let shortest = run
                .iter()
                .map(|(entry, _)| entry.length)
                .fold(f64::INFINITY, f64::min);
            for (entry, _) in run {
                let (piece, forward) = piece_of(entry.half_edge);
                let deviation = if run.len() > 1 {
                    let piece = pieces.get(piece).ok_or_else(ProfileError::unresolved)?;
                    probe_angle(piece, entry, forward, PROBE_FRACTION * shortest, base_angle)
                } else {
                    0.0
                };
                keyed.push((entry.half_edge, base_angle, deviation));
            }
            run_start = run_end;
        }
        keyed.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.2.total_cmp(&b.2)));
        let half_edges: Vec<usize> = keyed.iter().map(|(half_edge, _, _)| *half_edge).collect();
        for (rank, half_edge) in half_edges.iter().enumerate() {
            if let Some(slot) = position.get_mut(*half_edge) {
                *slot = rank;
            }
        }
        order.push(half_edges);
    }
    Ok((order, position))
}
