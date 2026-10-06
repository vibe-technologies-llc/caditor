use std::collections::{BTreeMap, BTreeSet};

use caditor_geometry::{Aabb, Point2, Point3};

use crate::{
    boolean::{BooleanError, FaceBounds, FaceKey, Input, Operand, TOLERANCE},
    curve::Curve,
    interrupt,
    intersect::{
        IntersectionBranch, IntersectionError, SurfaceIntersection, SurfacePatch,
        intersect_curve_surface, intersect_curves, intersect_surfaces_through,
    },
    interval::Interval,
    naming::EdgeName,
    sense::Sense,
    surface::Surface,
    topology::{EdgeId, FaceContainment, FaceId, Solid},
};

const SAME_EDGE: f64 = TOLERANCE;
const EDGE_SAMPLES: [f64; 3] = [0.25, 0.5, 0.75];
const INSIDE_SAMPLES: [f64; 5] = [0.5, 0.25, 0.75, 0.05, 0.95];
const RANGE_SLACK: f64 = 1e-9;
const CELLS_ACROSS: f64 = 256.0;
const CELL_TOLERANCES: f64 = 4.0;
const CELLS_PER_PIECE: f64 = 2.0;
const MAX_CURVE_PIECES: f64 = 256.0;
const UNJUDGED_PIECE: f64 = 2.0 * TOLERANCE;

type Mark = (f64, Option<usize>);

#[derive(Debug, Clone)]
pub(super) struct Source {
    pub curve: Curve,
    pub name: EdgeName,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Piece {
    pub source: usize,
    pub interval: Interval,
    pub start: usize,
    pub end: usize,
}

impl Piece {
    pub fn is_closed(&self) -> bool {
        self.start == self.end
    }

    pub fn start_of(&self, sense: Sense) -> usize {
        if sense.is_same() {
            self.start
        } else {
            self.end
        }
    }

    pub fn end_of(&self, sense: Sense) -> usize {
        self.start_of(sense.reversed())
    }
}

#[derive(Debug, Clone, Default)]
pub(super) struct Arrangement {
    points: Vec<Point3>,
    sources: Vec<Source>,
    pieces: Vec<Piece>,
    representatives: Vec<(usize, Sense)>,
    edge_pieces: BTreeMap<(Operand, EdgeId), Vec<usize>>,
    cuts: BTreeMap<FaceKey, Vec<usize>>,
}

impl Arrangement {
    pub fn point(&self, vertex: usize) -> Option<Point3> {
        self.points.get(vertex).copied()
    }

    pub fn piece(&self, piece: usize) -> Option<&Piece> {
        self.pieces.get(piece)
    }

    pub fn source(&self, source: usize) -> Option<&Source> {
        self.sources.get(source)
    }

    pub fn curve(&self, piece: usize) -> Option<(&Curve, &Piece)> {
        let data = self.pieces.get(piece)?;
        Some((&self.sources.get(data.source)?.curve, data))
    }

    pub fn representative(&self, piece: usize) -> (usize, Sense) {
        self.representatives
            .get(piece)
            .copied()
            .unwrap_or((piece, Sense::Same))
    }

    pub fn edge_pieces(&self, operand: Operand, edge: EdgeId) -> &[usize] {
        self.edge_pieces
            .get(&(operand, edge))
            .map_or(&[], Vec::as_slice)
    }

    pub fn cuts(&self, face: FaceKey) -> &[usize] {
        self.cuts.get(&face).map_or(&[], Vec::as_slice)
    }

    pub fn shared_pieces(&self) -> BTreeSet<usize> {
        let cut = self
            .cuts
            .values()
            .flatten()
            .map(|piece| self.representative(*piece).0);
        let merged = self
            .representatives
            .iter()
            .enumerate()
            .filter(|(piece, (representative, _))| piece != representative)
            .flat_map(|(piece, (representative, _))| [piece, *representative]);
        cut.chain(merged).collect()
    }

    pub fn add_source(&mut self, source: Source) -> usize {
        self.sources.push(source);
        self.sources.len() - 1
    }

    pub fn add_piece(&mut self, piece: Piece) -> usize {
        self.pieces.push(piece);
        self.representatives
            .push((self.pieces.len() - 1, Sense::Same));
        self.pieces.len() - 1
    }
}

type Cell = (i64, i64, i64);

#[derive(Debug, Clone)]
struct Pool {
    points: Vec<Point3>,
    owners: Vec<[bool; 2]>,
    cell: f64,
    cells: BTreeMap<Cell, Vec<usize>>,
}

fn owner_index(operand: Operand) -> usize {
    match operand {
        Operand::First => 0,
        Operand::Second => 1,
    }
}

impl Pool {
    fn new(extent: f64) -> Self {
        Self {
            points: Vec::new(),
            owners: Vec::new(),
            cell: (extent / CELLS_ACROSS).max(CELL_TOLERANCES * TOLERANCE),
            cells: BTreeMap::new(),
        }
    }

    fn cell_of(&self, point: Point3) -> Cell {
        let index = |value: f64| (value / self.cell).floor() as i64;
        (index(point.x), index(point.y), index(point.z))
    }

    fn in_box(&self, bounds: &Aabb) -> Vec<usize> {
        let (low, high) = (self.cell_of(bounds.min()), self.cell_of(bounds.max()));
        let span = |from: i64, to: i64| u128::from(to.abs_diff(from)) + 1;
        let cells = span(low.0, high.0) * span(low.1, high.1) * span(low.2, high.2);
        let inside = |cell: &Cell| {
            (low.0..=high.0).contains(&cell.0)
                && (low.1..=high.1).contains(&cell.1)
                && (low.2..=high.2).contains(&cell.2)
        };
        let mut found: Vec<usize> = if cells > self.cells.len() as u128 {
            self.cells
                .iter()
                .filter(|(cell, _)| inside(cell))
                .flat_map(|(_, members)| members.iter().copied())
                .collect()
        } else {
            let mut found = Vec::new();
            for x in low.0..=high.0 {
                for y in low.1..=high.1 {
                    for (_, members) in self.cells.range((x, y, low.2)..=(x, y, high.2)) {
                        found.extend(members.iter().copied());
                    }
                }
            }
            found
        };
        found.retain(|index| {
            self.points.get(*index).is_some_and(|point| {
                point.cmpge(bounds.min()).all() && point.cmple(bounds.max()).all()
            })
        });
        found
    }

    fn insert(&mut self, point: Point3, owner: Option<Operand>) -> usize {
        let existing = self
            .in_box(&Aabb::from_point(point).expanded(TOLERANCE))
            .into_iter()
            .filter_map(|index| {
                let known = self.points.get(index)?;
                Some((index, known.distance_squared(point)))
            })
            .filter(|(_, distance)| *distance <= TOLERANCE * TOLERANCE)
            .min_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)))
            .map(|(index, _)| index);
        let index = existing.unwrap_or_else(|| {
            self.points.push(point);
            self.owners.push([false; 2]);
            let index = self.points.len() - 1;
            self.cells
                .entry(self.cell_of(point))
                .or_default()
                .push(index);
            index
        });
        if let Some(owner) = owner
            && let Some(flags) = self.owners.get_mut(index)
            && let Some(flag) = flags.get_mut(owner_index(owner))
        {
            *flag = true;
        }
        index
    }

    fn owned_by(&self, vertex: usize, operand: Operand) -> bool {
        self.owners
            .get(vertex)
            .and_then(|flags| flags.get(owner_index(operand)))
            .copied()
            .unwrap_or(false)
    }

    fn near(&self, curve: &Curve, interval: Interval) -> Vec<(usize, Point3)> {
        let whole = curve.bounding_box(interval);
        let pieces = (whole.diagonal() / (CELLS_PER_PIECE * self.cell))
            .ceil()
            .clamp(1.0, MAX_CURVE_PIECES) as usize;
        let mut found: Vec<usize> = interval
            .split(pieces)
            .collect::<Vec<f64>>()
            .windows(2)
            .filter_map(|pair| Interval::new(*pair.first()?, *pair.get(1)?))
            .flat_map(|piece| self.in_box(&curve.bounding_box(piece).expanded(TOLERANCE)))
            .collect();
        found.sort_unstable();
        found.dedup();
        found
            .into_iter()
            .filter_map(|index| Some((index, *self.points.get(index)?)))
            .collect()
    }
}

#[derive(Debug, Clone, Copy)]
struct Overlap {
    edge: (Operand, EdgeId),
    face: FaceKey,
    range: Interval,
}

#[derive(Debug, Clone)]
struct Branch {
    faces: [FaceKey; 2],
    branch: IntersectionBranch,
}

fn touches(containment: Option<FaceContainment>) -> bool {
    matches!(
        containment,
        Some(FaceContainment::Inside | FaceContainment::OnBoundary)
    )
}

fn boundary_edges(solid: &Solid, face: FaceId) -> Vec<EdgeId> {
    let mut edges: Vec<EdgeId> = solid
        .face(face)
        .into_iter()
        .flat_map(|face| face.loops().iter())
        .filter_map(|loop_id| solid.face_loop(*loop_id))
        .flat_map(|face_loop| face_loop.coedges().iter())
        .filter_map(|coedge| solid.coedge(*coedge).map(|coedge| coedge.edge()))
        .collect();
    edges.sort_unstable();
    edges.dedup();
    edges
}

pub(super) fn imprint(input: &Input) -> Result<Arrangement, BooleanError> {
    let extent = Operand::BOTH
        .iter()
        .flat_map(|operand| input.faces(*operand))
        .map(|face| face.bounds)
        .reduce(Aabb::union)
        .map_or(1.0, |bounds| bounds.diagonal());
    let mut pool = Pool::new(extent);
    let vertex_ids = Operand::BOTH.map(|operand| {
        input
            .solid(operand)
            .vertices()
            .map(|(_, vertex)| pool.insert(vertex.point(), Some(operand)))
            .collect::<Vec<usize>>()
    });
    let mut overlaps = Vec::new();
    for operand in Operand::BOTH {
        edge_hits(input, operand, &mut pool, &mut overlaps)?;
    }
    let branches = face_branches(input, &mut pool)?;
    let mut arrangement = Arrangement::default();
    let mut merged = Merged::default();
    for (operand, ids) in Operand::BOTH.into_iter().zip(&vertex_ids) {
        split_edges(input, operand, ids, &pool, &mut arrangement, &mut merged)?;
    }
    for overlap in &overlaps {
        add_overlap_cuts(input, overlap, &mut arrangement);
    }
    for branch in &branches {
        interrupt::check()?;
        clip_branch(input, branch, &mut pool, &mut arrangement);
    }
    for piece in &mut arrangement.pieces {
        piece.start = merged.find(piece.start);
        piece.end = merged.find(piece.end);
    }
    arrangement.points = pool.points;
    arrangement.representatives = deduplicate(&arrangement);
    Ok(arrangement)
}

fn edge_hits(
    input: &Input,
    operand: Operand,
    pool: &mut Pool,
    overlaps: &mut Vec<Overlap>,
) -> Result<(), BooleanError> {
    let other = operand.other();
    let solid = input.solid(operand);
    let target = input.solid(other);
    let classifier = input.classifier(other);
    for (edge_id, edge) in solid.edges() {
        interrupt::check()?;
        let curve = edge.curve();
        let bounds = curve.bounding_box(edge.interval());
        for face in input.faces_near(other, &bounds) {
            let Some(surface) = target.face(face.id).map(|face| face.surface()) else {
                continue;
            };
            let located = |error: IntersectionError| {
                let key = FaceKey {
                    operand: other,
                    face: face.id,
                };
                BooleanError::from(error)
                    .or_faces(
                        edge_face_keys(solid, operand, edge_id)
                            .into_iter()
                            .chain([key]),
                    )
                    .or_point(|| {
                        let nearest =
                            curve.closest_parameter(face.bounds.center(), edge.interval());
                        Some(curve.point(nearest))
                    })
            };
            let found = intersect_curve_surface(curve, edge.interval(), surface, Some(face.uv))
                .map_err(located)?;
            for hit in &found.points {
                if touches(classifier.point_in_face(face.id, hit.uv)) {
                    pool.insert(hit.point, None);
                }
            }
            for overlap in &found.overlaps {
                for (parameter, uv) in [
                    (overlap.range.start(), overlap.start_uv),
                    (overlap.range.end(), overlap.end_uv),
                ] {
                    if touches(classifier.point_in_face(face.id, uv)) {
                        pool.insert(curve.point(parameter), None);
                    }
                }
                for boundary in boundary_edges(target, face.id) {
                    let Some(boundary) = target.edge(boundary) else {
                        continue;
                    };
                    let crossings = intersect_curves(
                        curve,
                        overlap.range,
                        boundary.curve(),
                        boundary.interval(),
                    )
                    .map_err(located)?;
                    for crossing in crossings.points {
                        pool.insert(crossing.point, None);
                    }
                }
                overlaps.push(Overlap {
                    edge: (operand, edge_id),
                    face: FaceKey {
                        operand: other,
                        face: face.id,
                    },
                    range: overlap.range,
                });
            }
        }
    }
    Ok(())
}

fn edge_face_keys(solid: &Solid, operand: Operand, edge: EdgeId) -> Vec<FaceKey> {
    solid
        .edge(edge)
        .into_iter()
        .flat_map(|edge| edge.coedges().iter())
        .filter_map(|coedge| solid.coedge_face(*coedge))
        .map(|face| FaceKey { operand, face })
        .collect()
}

fn meeting_point(first: &FaceBounds, second: &FaceBounds, surface: &Surface) -> Point3 {
    let (low, high) = (
        first.bounds.min().max(second.bounds.min()),
        first.bounds.max().min(second.bounds.max()),
    );
    let middle = (low + high) * 0.5;
    surface.point_at(surface.project(middle, Some(first.uv.center())))
}

fn shared_points(pool: &Pool, faces: [&FaceBounds; 2], surfaces: [&Surface; 2]) -> Vec<Point3> {
    let [first, second] = faces;
    let low = first.bounds.min().max(second.bounds.min());
    let high = first.bounds.max().min(second.bounds.max());
    if low.cmpgt(high).any() {
        return Vec::new();
    }
    let shared = Aabb::from_point(low).including(high);
    pool.in_box(&shared)
        .into_iter()
        .filter_map(|index| pool.points.get(index).copied())
        .filter(|point| {
            surfaces
                .iter()
                .all(|surface| surface.distance(*point) <= SAME_EDGE)
        })
        .collect()
}

fn face_branches(input: &Input, pool: &mut Pool) -> Result<Vec<Branch>, BooleanError> {
    let mut branches = Vec::new();
    for first in input.faces(Operand::First) {
        for second in input.faces_near(Operand::Second, &first.bounds) {
            interrupt::check()?;
            let (Some(first_face), Some(second_face)) =
                (input.first.face(first.id), input.second.face(second.id))
            else {
                continue;
            };
            let faces = [
                FaceKey {
                    operand: Operand::First,
                    face: first.id,
                },
                FaceKey {
                    operand: Operand::Second,
                    face: second.id,
                },
            ];
            let hints = shared_points(
                pool,
                [first, second],
                [first_face.surface(), second_face.surface()],
            );
            let found = SurfacePatch::new(first_face.surface(), first.uv)
                .and_then(|first_patch| {
                    let second_patch = SurfacePatch::new(second_face.surface(), second.uv)?;
                    intersect_surfaces_through(&first_patch, &second_patch, &hints)
                })
                .map_err(|error| {
                    BooleanError::from(error)
                        .or_faces(faces)
                        .or_point(|| Some(meeting_point(first, second, first_face.surface())))
                })?;
            let SurfaceIntersection::Branches {
                branches: found,
                points,
            } = found
            else {
                continue;
            };
            for point in points {
                let [first_uv, second_uv] = point.uv;
                let inside = touches(
                    input
                        .classifier(Operand::First)
                        .point_in_face(first.id, first_uv),
                ) && touches(
                    input
                        .classifier(Operand::Second)
                        .point_in_face(second.id, second_uv),
                );
                if inside {
                    pool.insert(point.point, None);
                }
            }
            branches.extend(found.into_iter().map(|branch| Branch { faces, branch }));
        }
    }
    Ok(branches)
}

#[derive(Debug, Clone, Default)]
struct Merged {
    parents: BTreeMap<usize, usize>,
}

impl Merged {
    fn find(&self, vertex: usize) -> usize {
        let mut current = vertex;
        let mut steps = 0;
        while let Some(parent) = self.parents.get(&current)
            && *parent != current
            && steps <= self.parents.len()
        {
            current = *parent;
            steps += 1;
        }
        current
    }

    fn join(&mut self, first: usize, second: usize) {
        let (first, second) = (self.find(first), self.find(second));
        if first != second {
            self.parents.insert(first.max(second), first.min(second));
        }
    }
}

fn split_edges(
    input: &Input,
    operand: Operand,
    vertex_ids: &[usize],
    pool: &Pool,
    arrangement: &mut Arrangement,
    merged: &mut Merged,
) -> Result<(), BooleanError> {
    let solid = input.solid(operand);
    for (edge_id, edge) in solid.edges() {
        interrupt::check()?;
        let curve = edge.curve();
        let interval = edge.interval();
        let (Some(start), Some(end)) = (
            vertex_ids.get(edge.start().index()).copied(),
            vertex_ids.get(edge.end().index()).copied(),
        ) else {
            return Err(BooleanError::split()
                .or_faces(edge_face_keys(solid, operand, edge_id))
                .or_point(|| Some(curve.point(interval.middle()))));
        };
        let mut stops = vec![(interval.start(), start), (interval.end(), end)];
        for (vertex, point) in pool.near(curve, interval) {
            if pool.owned_by(vertex, operand) {
                continue;
            }
            let parameter = curve.closest_parameter(point, interval);
            let inside = parameter > interval.start() && parameter < interval.end();
            if inside && curve.point(parameter).distance(point) <= TOLERANCE {
                stops.push((parameter, vertex));
            }
        }
        stops.sort_by(|a, b| a.0.total_cmp(&b.0));
        let source = arrangement.add_source(Source {
            curve: curve.clone(),
            name: edge.name(),
        });
        let mut pieces = Vec::with_capacity(stops.len());
        for pair in stops.windows(2) {
            let [(from, start), (to, end)] = pair else {
                continue;
            };
            let Some(range) = Interval::new(*from, *to) else {
                merged.join(*start, *end);
                continue;
            };
            if curve.length(range) <= TOLERANCE {
                merged.join(*start, *end);
                continue;
            }
            pieces.push(arrangement.add_piece(Piece {
                source,
                interval: range,
                start: *start,
                end: *end,
            }));
        }
        arrangement.edge_pieces.insert((operand, edge_id), pieces);
    }
    Ok(())
}

fn lies_on(surface: &Surface, curve: &Curve, range: Interval) -> bool {
    EDGE_SAMPLES
        .iter()
        .all(|fraction| surface.distance(curve.point(range.at(*fraction))) <= TOLERANCE)
}

fn add_overlap_cuts(input: &Input, overlap: &Overlap, arrangement: &mut Arrangement) {
    let (Some(face), Some(bounds)) = (input.face(overlap.face), input.bounds(overlap.face)) else {
        return;
    };
    let surface = face.surface();
    let classifier = input.classifier(overlap.face.operand);
    let slack = RANGE_SLACK * (1.0 + overlap.range.end().abs());
    let mut cuts = Vec::new();
    for piece in arrangement.edge_pieces(overlap.edge.0, overlap.edge.1) {
        let Some((curve, data)) = arrangement.curve(*piece) else {
            continue;
        };
        let middle = data.interval.middle();
        if middle < overlap.range.start() - slack || middle > overlap.range.end() + slack {
            continue;
        }
        if !lies_on(surface, curve, data.interval) {
            continue;
        }
        let uv = surface.project(curve.point(middle), Some(bounds.uv.center()));
        if classifier.point_in_face(overlap.face.face, uv) == Some(FaceContainment::Inside) {
            cuts.push(*piece);
        }
    }
    arrangement
        .cuts
        .entry(overlap.face)
        .or_default()
        .extend(cuts);
}

fn face_uvs(
    input: &Input,
    branch: &Branch,
    parameter: f64,
    point: Point3,
) -> Option<[(FaceKey, Point2); 2]> {
    let hints = match &branch.branch.curve {
        Curve::Intersection(curve) => curve.uv_at(parameter).map(Some),
        _ => [None, None],
    };
    let mut located = [(branch.faces[0], Point2::ZERO); 2];
    for ((slot, key), hint) in located.iter_mut().zip(branch.faces).zip(hints) {
        let surface = input.face(key)?.surface();
        let hint = hint.or_else(|| input.bounds(key).map(|bounds| bounds.uv.center()));
        *slot = (key, surface.project(point, hint));
    }
    Some(located)
}

fn inside_both(input: &Input, branch: &Branch, interval: Interval, length: f64) -> bool {
    let mut inside = [false; 2];
    for fraction in INSIDE_SAMPLES {
        let parameter = interval.at(fraction);
        let point = branch.branch.curve.point(parameter);
        let Some(located) = face_uvs(input, branch, parameter, point) else {
            return false;
        };
        for (seen, (key, uv)) in inside.iter_mut().zip(located) {
            match input.classifier(key.operand).point_in_face(key.face, uv) {
                Some(FaceContainment::Inside) => *seen = true,
                Some(FaceContainment::OnBoundary) => {}
                Some(FaceContainment::Outside) | None => return false,
            }
        }
        if inside.iter().all(|seen| *seen) {
            return true;
        }
    }
    length <= UNJUDGED_PIECE
}

fn branch_stops(curve: &Curve, range: Interval, closed: bool, pool: &Pool) -> Vec<(f64, usize)> {
    let mut stops: Vec<(f64, usize)> = Vec::new();
    for (vertex, point) in pool.near(curve, range) {
        let parameter = curve.closest_parameter(point, range);
        if curve.point(parameter).distance(point) > TOLERANCE {
            continue;
        }
        let parameter = if closed && curve.point(range.start()).distance(point) <= TOLERANCE {
            range.start()
        } else {
            parameter
        };
        stops.push((parameter, vertex));
    }
    stops.sort_by(|a, b| a.0.total_cmp(&b.0));
    stops.dedup_by_key(|stop| stop.1);
    stops
}

fn clip_branch(input: &Input, branch: &Branch, pool: &mut Pool, arrangement: &mut Arrangement) {
    let curve = &branch.branch.curve;
    let range = branch.branch.range;
    let closed = branch.branch.closed;
    let mut stops = branch_stops(curve, range, closed, pool);
    let segments: Vec<(Mark, Mark)> = if closed {
        if stops.is_empty() {
            let vertex = pool.insert(curve.point(range.start()), None);
            stops.push((range.start(), vertex));
        }
        let period = range.length();
        let wrap = stops
            .first()
            .map(|(parameter, vertex)| (parameter + period, Some(*vertex)));
        stops
            .iter()
            .map(|(parameter, vertex)| (*parameter, Some(*vertex)))
            .chain(wrap)
            .collect::<Vec<_>>()
            .windows(2)
            .filter_map(|pair| match pair {
                [a, b] => Some((*a, *b)),
                _ => None,
            })
            .collect()
    } else {
        let mut marks: Vec<Mark> = Vec::with_capacity(stops.len() + 2);
        let covered = |parameter: f64| {
            let point = curve.point(parameter);
            stops.iter().any(|(_, vertex)| {
                pool.points
                    .get(*vertex)
                    .is_some_and(|known| known.distance(point) <= TOLERANCE)
            })
        };
        if !covered(range.start()) {
            marks.push((range.start(), None));
        }
        marks.extend(
            stops
                .iter()
                .map(|(parameter, vertex)| (*parameter, Some(*vertex))),
        );
        if !covered(range.end()) {
            marks.push((range.end(), None));
        }
        marks
            .windows(2)
            .filter_map(|pair| match pair {
                [a, b] => Some((*a, *b)),
                _ => None,
            })
            .collect()
    };
    let mut source = None;
    for ((from, start), (to, end)) in segments {
        let Some(interval) = Interval::new(from, to) else {
            continue;
        };
        let length = curve.length(interval);
        if length <= TOLERANCE || !inside_both(input, branch, interval, length) {
            continue;
        }
        let start = start.unwrap_or_else(|| pool.insert(curve.point(from), None));
        let end = end.unwrap_or_else(|| pool.insert(curve.point(to), None));
        let source = *source.get_or_insert_with(|| {
            let names = branch
                .faces
                .map(|key| input.face(key).map(|face| face.name()).unwrap_or_default());
            arrangement.add_source(Source {
                curve: curve.clone(),
                name: EdgeName::between(names[0], names[1]),
            })
        });
        let piece = arrangement.add_piece(Piece {
            source,
            interval,
            start,
            end,
        });
        for key in branch.faces {
            arrangement.cuts.entry(key).or_default().push(piece);
        }
    }
}

fn same_piece(arrangement: &Arrangement, first: usize, second: usize) -> Option<Sense> {
    let (first_curve, first_piece) = arrangement.curve(first)?;
    let (second_curve, second_piece) = arrangement.curve(second)?;
    let mut relation = None;
    for fraction in EDGE_SAMPLES {
        let parameter = first_piece.interval.at(fraction);
        let point = first_curve.point(parameter);
        let on_second = second_curve.closest_parameter(point, second_piece.interval);
        if second_curve.point(on_second).distance(point) > SAME_EDGE {
            return None;
        }
        if relation.is_none() {
            let along = first_curve
                .evaluate(parameter)
                .first
                .dot(second_curve.evaluate(on_second).first);
            relation = Some(Sense::from_sign(along));
        }
    }
    let relation = relation?;
    if first_piece.is_closed() {
        return Some(relation);
    }
    let expected = if relation.is_same() {
        (second_piece.start, second_piece.end)
    } else {
        (second_piece.end, second_piece.start)
    };
    (expected == (first_piece.start, first_piece.end)).then_some(relation)
}

fn deduplicate(arrangement: &Arrangement) -> Vec<(usize, Sense)> {
    let count = arrangement.pieces.len();
    let mut representatives: Vec<(usize, Sense)> =
        (0..count).map(|index| (index, Sense::Same)).collect();
    let mut groups: BTreeMap<(usize, usize), Vec<usize>> = BTreeMap::new();
    for (index, piece) in arrangement.pieces.iter().enumerate() {
        let key = (piece.start.min(piece.end), piece.start.max(piece.end));
        groups.entry(key).or_default().push(index);
    }
    for members in groups.values() {
        for (position, first) in members.iter().enumerate() {
            if representatives
                .get(*first)
                .is_none_or(|(rep, _)| rep != first)
            {
                continue;
            }
            for second in members.iter().skip(position + 1) {
                if representatives
                    .get(*second)
                    .is_none_or(|(rep, _)| rep != second)
                {
                    continue;
                }
                if let Some(relation) = same_piece(arrangement, *second, *first)
                    && let Some(slot) = representatives.get_mut(*second)
                {
                    *slot = (*first, relation);
                }
            }
        }
    }
    representatives
}
