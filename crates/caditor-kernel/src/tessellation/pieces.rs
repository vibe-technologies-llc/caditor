use std::{collections::VecDeque, ops::Range};

use spade::{
    Point2 as PlanePoint, Triangulation,
    handles::{FixedFaceHandle, InnerTag},
};

use crate::{
    interrupt::{self, Interrupted},
    tessellation::{
        POLL_EVERY, TessellationError,
        constrained::{self, Built, Outline, Splitting},
        parallel,
    },
    topology::FaceId,
};

pub(super) const SPLIT_POINTS: usize = 4096;
const POINTS_PER_PIECE: usize = 1024;
const MAX_PIECES: usize = 16;
const REACH_SHARE: usize = 3;
const SLACK_SHARE: usize = 2;
const DOMINANT_SHARE: usize = 4;
const CENTRE_ERROR: f64 = 16.0;
const BAND_SHARE: f64 = 1e-9;
const TURN_BOUND: f64 = 4.0 * (3.0 + 16.0 * f64::EPSILON) * f64::EPSILON;
const IN_CIRCLE_BOUND: f64 = 4.0 * (10.0 + 96.0 * f64::EPSILON) * f64::EPSILON;

#[derive(Debug, Clone, Copy, PartialEq)]
struct Span {
    low: f64,
    high: f64,
}

impl Span {
    fn holds(&self, x: f64) -> bool {
        self.low <= x && x <= self.high
    }

    fn overlaps(&self, a: f64, b: f64) -> bool {
        a.min(b) <= self.high && a.max(b) >= self.low
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Strip {
    core: Span,
    reach: Span,
    band: f64,
}

impl Strip {
    fn clear_of_cuts(&self, x: f64) -> bool {
        self.core.low + self.band < x && x + self.band < self.core.high
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Circle {
    centre: PlanePoint<f64>,
    radius: f64,
    error: f64,
}

impl Circle {
    fn through(corners: [PlanePoint<f64>; 3]) -> Option<Self> {
        let [a, b, c] = corners;
        let (bx, by, cx, cy) = (b.x - a.x, b.y - a.y, c.x - a.x, c.y - a.y);
        let twice = 2.0 * (bx * cy - by * cx);
        let (b_square, c_square) = (bx * bx + by * by, cx * cx + cy * cy);
        let centre_x = (cy * b_square - by * c_square) / twice;
        let centre_y = (bx * c_square - cx * b_square) / twice;
        let radius = centre_x.hypot(centre_y);
        let centre = PlanePoint::new(a.x + centre_x, a.y + centre_y);
        let spread = cy.abs() * b_square
            + by.abs() * c_square
            + centre_x.abs() * 2.0 * ((bx * cy).abs() + (by * cx).abs());
        let error = CENTRE_ERROR * f64::EPSILON * (spread / twice.abs() + centre.x.abs());
        (radius.is_finite() && centre.x.is_finite() && centre.y.is_finite() && error.is_finite())
            .then_some(Self {
                centre,
                radius,
                error,
            })
    }

    fn within(&self, span: &Span) -> bool {
        span.low < self.centre.x - self.radius && self.centre.x + self.radius < span.high
    }
}

#[derive(Debug, Default)]
struct Piece {
    triangles: Vec<[usize; 3]>,
    border: Vec<(usize, usize)>,
    covered: Vec<usize>,
}

pub(super) fn allowed(points: usize, body_points: usize) -> bool {
    points >= SPLIT_POINTS && points.saturating_mul(DOMINANT_SHARE) >= body_points
}

pub(super) fn triangles(
    face: FaceId,
    mapped: &[PlanePoint<f64>],
    outline: &Outline,
    threads: usize,
) -> Result<Option<Vec<[usize; 3]>>, Interrupted> {
    let strips = strips(mapped, outline);
    if strips.len() < 2 {
        return Ok(None);
    }
    match in_pieces(face, mapped, outline, &strips, threads) {
        Ok(triangles) => Ok(Some(triangles)),
        Err(TessellationError::Cancelled(interrupted)) => Err(interrupted),
        Err(_) => Ok(None),
    }
}

struct Ranked {
    xs: Vec<f64>,
    hole_lows: Vec<f64>,
    hole_highs: Vec<f64>,
}

impl Ranked {
    fn new(mapped: &[PlanePoint<f64>], outline: &Outline) -> Self {
        let sorted = |values: Vec<f64>| {
            let mut values = values;
            values.sort_by(f64::total_cmp);
            values
        };
        Self {
            xs: sorted(mapped.iter().map(|point| point.x).collect()),
            hole_lows: sorted(outline.holes.iter().map(|hole| hole.low).collect()),
            hole_highs: sorted(outline.holes.iter().map(|hole| hole.high).collect()),
        }
    }

    fn at(&self, rank: usize) -> Option<f64> {
        self.xs
            .get(rank.min(self.xs.len().saturating_sub(1)))
            .copied()
    }

    fn between(&self, rank: usize) -> Option<f64> {
        let middle = 0.5 * (self.at(rank.checked_sub(1)?)? + self.at(rank)?);
        middle.is_finite().then_some(middle)
    }

    fn straddling(&self, x: f64) -> usize {
        let opened = self.hole_lows.partition_point(|low| *low < x);
        let closed = self.hole_highs.partition_point(|high| *high < x);
        opened.saturating_sub(closed)
    }

    fn clearest(&self, ranks: Range<usize>) -> Option<f64> {
        ranks
            .filter_map(|rank| {
                let (before, after) = (self.at(rank.checked_sub(1)?)?, self.at(rank)?);
                (before < after).then(|| {
                    (
                        self.straddling(0.5 * (before + after)),
                        before - after,
                        rank,
                    )
                })
            })
            .min_by(|one, other| {
                one.0
                    .cmp(&other.0)
                    .then(one.1.total_cmp(&other.1))
                    .then(one.2.cmp(&other.2))
            })
            .and_then(|(_, _, rank)| self.between(rank))
    }
}

fn strips(mapped: &[PlanePoint<f64>], outline: &Outline) -> Vec<Strip> {
    let count = mapped.len();
    let pieces = (count / POINTS_PER_PIECE).min(MAX_PIECES);
    if count < SPLIT_POINTS || pieces < 2 {
        return Vec::new();
    }
    let ranked = Ranked::new(mapped, outline);
    let reach = count / pieces / REACH_SHARE;
    let slack = reach / SLACK_SHARE;
    let band = match (ranked.xs.first(), ranked.xs.last()) {
        (Some(low), Some(high)) => BAND_SHARE * (high - low),
        _ => return Vec::new(),
    };
    let mut cuts: Vec<(f64, usize)> = vec![(f64::NEG_INFINITY, 0)];
    for piece in 1..pieces {
        let rank = piece * count / pieces;
        let Some(cut) = ranked.between(rank) else {
            return Vec::new();
        };
        if cuts.last().is_some_and(|(last, _)| *last < cut) {
            cuts.push((cut, rank));
        }
    }
    cuts.push((f64::INFINITY, count));
    cuts.windows(2)
        .filter_map(|pair| match pair {
            [(low, first), (high, end)] => {
                let below = first.saturating_sub(reach);
                let above = (end + reach).min(count);
                Some(Strip {
                    core: Span {
                        low: *low,
                        high: *high,
                    },
                    reach: Span {
                        low: if low.is_finite() {
                            ranked
                                .clearest(below.saturating_sub(slack)..(below + slack).min(*first))
                                .or_else(|| ranked.at(below))?
                                .min(*low)
                        } else {
                            f64::NEG_INFINITY
                        },
                        high: if high.is_finite() {
                            ranked
                                .clearest(
                                    above.saturating_sub(slack).max(*end)
                                        ..(above + slack).min(count),
                                )
                                .or_else(|| ranked.at(above))?
                                .max(*high)
                        } else {
                            f64::INFINITY
                        },
                    },
                    band,
                })
            }
            _ => None,
        })
        .collect()
}

fn in_pieces(
    face: FaceId,
    mapped: &[PlanePoint<f64>],
    outline: &Outline,
    strips: &[Strip],
    threads: usize,
) -> Result<Vec<[usize; 3]>, TessellationError> {
    let pieces = parallel::each_in_order(strips.len(), threads, 1, |index| {
        piece(face, mapped, outline, strips, index)
    });
    let pieces = pieces.into_iter().collect::<Result<Vec<Piece>, _>>()?;
    let remainder = remainder(face, mapped, outline, &pieces)?;
    let mut triangles: Vec<[usize; 3]> = pieces
        .into_iter()
        .flat_map(|piece| piece.triangles)
        .collect();
    triangles.extend(remainder);
    Ok(triangles)
}

fn point(
    mapped: &[PlanePoint<f64>],
    index: usize,
    face: FaceId,
) -> Result<PlanePoint<f64>, TessellationError> {
    mapped
        .get(index)
        .copied()
        .ok_or(TessellationError::Triangulation(face))
}

fn core_of(strips: &[Strip], x: f64) -> usize {
    strips.partition_point(|strip| strip.core.high <= x)
}

fn piece(
    face: FaceId,
    mapped: &[PlanePoint<f64>],
    outline: &Outline,
    strips: &[Strip],
    index: usize,
) -> Result<Piece, TessellationError> {
    interrupt::check()?;
    let strip = strips
        .get(index)
        .ok_or(TessellationError::Triangulation(face))?;
    let mut member: Vec<bool> = mapped.iter().map(|at| strip.reach.holds(at.x)).collect();
    let mut crossing = Vec::new();
    let mut crossing_segments = Vec::new();
    for (step, (from, to)) in outline.segments.iter().enumerate() {
        if step.is_multiple_of(POLL_EVERY) {
            interrupt::check()?;
        }
        let (a, b) = (point(mapped, *from, face)?, point(mapped, *to, face)?);
        if strip.reach.overlaps(a.x, b.x) {
            crossing.push((*from, *to));
            crossing_segments.push(step);
            for end in [*from, *to] {
                if let Some(slot) = member.get_mut(end) {
                    *slot = true;
                }
            }
        }
    }
    let members: Vec<usize> = member
        .iter()
        .enumerate()
        .filter(|(_, member)| **member)
        .map(|(index, _)| index)
        .collect();
    let helpers: Vec<PlanePoint<f64>> = outline
        .holes
        .iter()
        .filter(|hole| strip.reach.holds(hole.low) && strip.reach.holds(hole.high))
        .map(|hole| hole.centre)
        .collect();
    let built = constrained::build(
        face,
        mapped,
        members,
        &crossing,
        Splitting::Allowed,
        &helpers,
    )?;
    let within = |fixed: FixedFaceHandle<InnerTag>| {
        built
            .cdt
            .face(fixed)
            .positions()
            .iter()
            .all(|at| strip.reach.holds(at.x))
    };
    let inside = inside_within(&built, mapped, &crossing, face, &within)?;
    let mut kept = vec![false; built.cdt.num_all_faces()];
    for fixed in built.cdt.fixed_inner_faces() {
        let here = inside.get(fixed.index()).copied().unwrap_or(false);
        let Some(corners) = built.corners(fixed) else {
            if here {
                return Err(TessellationError::Triangulation(face));
            }
            continue;
        };
        if let Some(slot) = kept.get_mut(fixed.index()) {
            *slot = here && keeps(&built, mapped, strips, index, fixed, corners);
        }
    }
    let is_kept =
        |fixed: FixedFaceHandle<InnerTag>| kept.get(fixed.index()).copied().unwrap_or(false);
    let mut made = Piece::default();
    for (step, face_handle) in built.cdt.inner_faces().enumerate() {
        if step.is_multiple_of(POLL_EVERY) {
            interrupt::check()?;
        }
        if !is_kept(face_handle.fix()) {
            continue;
        }
        made.triangles.push(
            built
                .corners(face_handle.fix())
                .ok_or(TessellationError::Triangulation(face))?,
        );
        for edge in face_handle.adjacent_edges() {
            let (Some(from), Some(to)) = (
                built.member(edge.from().fix()),
                built.member(edge.to().fix()),
            ) else {
                return Err(TessellationError::Triangulation(face));
            };
            if !built.cdt.is_constraint_edge(edge.fix().as_undirected())
                && !edge
                    .rev()
                    .face()
                    .as_inner()
                    .is_some_and(|neighbour| is_kept(neighbour.fix()))
            {
                made.border.push((from, to));
            }
        }
    }
    for (segment, (from, to)) in crossing_segments.iter().zip(&crossing) {
        let edge = built
            .handle(*from)
            .zip(built.handle(*to))
            .and_then(|(from, to)| built.cdt.get_edge_from_neighbors(from, to));
        let covers = edge.is_some_and(|edge| {
            [edge.face(), edge.rev().face()]
                .iter()
                .any(|side| side.as_inner().is_some_and(|inner| is_kept(inner.fix())))
        });
        if covers {
            made.covered.push(*segment);
        }
    }
    Ok(made)
}

fn keeps(
    built: &Built,
    mapped: &[PlanePoint<f64>],
    strips: &[Strip],
    index: usize,
    fixed: FixedFaceHandle<InnerTag>,
    corners: [usize; 3],
) -> bool {
    let mut canonical = corners;
    canonical.sort_unstable();
    let [Some(a), Some(b), Some(c)] = canonical.map(|corner| mapped.get(corner).copied()) else {
        return false;
    };
    let (Some(strip), Some(circle)) = (strips.get(index), Circle::through([a, b, c])) else {
        return false;
    };
    if core_of(strips, circle.centre.x) != index || !circle.within(&strip.reach) {
        return false;
    }
    let settled = circle.error * 4.0 <= strip.band && strip.clear_of_cuts(circle.centre.x);
    settled || strictly_delaunay(built, fixed)
}

fn strictly_delaunay(built: &Built, fixed: FixedFaceHandle<InnerTag>) -> bool {
    let face = built.cdt.face(fixed);
    let corners = face.positions();
    face.adjacent_edges().iter().all(|edge| {
        if built.cdt.is_constraint_edge(edge.fix().as_undirected()) {
            return true;
        }
        let across = edge.rev();
        if across.face().is_outer() {
            return true;
        }
        let apex = across.next().to();
        built.member(apex.fix()).is_some() && strictly_outside(corners, apex.position())
    })
}

fn turning(corners: [PlanePoint<f64>; 3]) -> f64 {
    let [a, b, c] = corners;
    let (left, right) = ((b.x - a.x) * (c.y - a.y), (b.y - a.y) * (c.x - a.x));
    let turn = left - right;
    if turn.abs() > TURN_BOUND * (left.abs() + right.abs()) {
        turn.signum()
    } else {
        0.0
    }
}

fn strictly_outside(corners: [PlanePoint<f64>; 3], at: PlanePoint<f64>) -> bool {
    let turn = turning(corners);
    let [a, b, c] = corners.map(|corner| (corner.x - at.x, corner.y - at.y));
    let lift = |(x, y): (f64, f64)| x * x + y * y;
    let (bc, cb) = (b.0 * c.1, c.0 * b.1);
    let (ca, ac) = (c.0 * a.1, a.0 * c.1);
    let (ab, ba) = (a.0 * b.1, b.0 * a.1);
    let determinant = lift(a) * (bc - cb) + lift(b) * (ca - ac) + lift(c) * (ab - ba);
    let permanent = (bc.abs() + cb.abs()) * lift(a)
        + (ca.abs() + ac.abs()) * lift(b)
        + (ab.abs() + ba.abs()) * lift(c);
    turn * determinant < -IN_CIRCLE_BOUND * permanent
}

fn inside_within(
    built: &Built,
    mapped: &[PlanePoint<f64>],
    crossing: &[(usize, usize)],
    face: FaceId,
    within: &impl Fn(FixedFaceHandle<InnerTag>) -> bool,
) -> Result<Vec<bool>, TessellationError> {
    let cdt = &built.cdt;
    let mut parity: Vec<Option<bool>> = vec![None; cdt.num_all_faces()];
    for (step, fixed) in cdt.fixed_inner_faces().enumerate() {
        if step.is_multiple_of(POLL_EVERY) {
            interrupt::check()?;
        }
        if parity.get(fixed.index()).copied().flatten().is_some() || !within(fixed) {
            continue;
        }
        let corners = cdt.face(fixed).positions();
        let centroid = PlanePoint::new(
            corners.iter().map(|at| at.x / 3.0).sum(),
            corners.iter().map(|at| at.y / 3.0).sum(),
        );
        let seed = encloses(mapped, crossing, centroid, face)?;
        if let Some(slot) = parity.get_mut(fixed.index()) {
            *slot = Some(seed);
        }
        constrained::spread_parity(cdt, &mut parity, VecDeque::from([fixed]), within);
    }
    Ok(parity
        .into_iter()
        .map(|inside| inside.unwrap_or(false))
        .collect())
}

fn encloses(
    mapped: &[PlanePoint<f64>],
    crossing: &[(usize, usize)],
    at: PlanePoint<f64>,
    face: FaceId,
) -> Result<bool, TessellationError> {
    let mut inside = false;
    for (from, to) in crossing {
        let (a, b) = (point(mapped, *from, face)?, point(mapped, *to, face)?);
        if (a.x <= at.x) == (b.x <= at.x) {
            continue;
        }
        let y = a.y + (at.x - a.x) * (b.y - a.y) / (b.x - a.x);
        if y > at.y {
            inside = !inside;
        }
    }
    Ok(inside)
}

fn remainder(
    face: FaceId,
    mapped: &[PlanePoint<f64>],
    outline: &Outline,
    pieces: &[Piece],
) -> Result<Vec<[usize; 3]>, TessellationError> {
    interrupt::check()?;
    let mut covered = vec![false; outline.segments.len()];
    for segment in pieces.iter().flat_map(|piece| piece.covered.iter()) {
        if let Some(slot) = covered.get_mut(*segment) {
            *slot = true;
        }
    }
    let is_covered = |segment: usize| covered.get(segment).copied().unwrap_or(false);
    let mut used = vec![false; mapped.len()];
    for corner in pieces
        .iter()
        .flat_map(|piece| piece.triangles.iter().flatten())
    {
        if let Some(slot) = used.get_mut(*corner) {
            *slot = true;
        }
    }
    let mut borders: Vec<(usize, usize)> = pieces
        .iter()
        .flat_map(|piece| piece.border.iter().copied())
        .collect();
    borders.sort_unstable();
    let mut constraints: Vec<(usize, usize)> = borders
        .iter()
        .filter(|(from, to)| borders.binary_search(&(*to, *from)).is_err())
        .copied()
        .collect();
    constraints.extend(
        outline
            .segments
            .iter()
            .enumerate()
            .filter(|(segment, _)| !is_covered(*segment))
            .map(|(_, segment)| *segment),
    );
    let mut member: Vec<bool> = used.iter().map(|used| !used).collect();
    for end in constraints.iter().flat_map(|(from, to)| [*from, *to]) {
        if let Some(slot) = member.get_mut(end) {
            *slot = true;
        }
    }
    let members: Vec<usize> = member
        .iter()
        .enumerate()
        .filter(|(_, member)| **member)
        .map(|(index, _)| index)
        .collect();
    let helpers: Vec<PlanePoint<f64>> = outline
        .holes
        .iter()
        .filter(|hole| hole.segments.clone().any(|segment| !is_covered(segment)))
        .map(|hole| hole.centre)
        .collect();
    let built = constrained::build(
        face,
        mapped,
        members,
        &constraints,
        Splitting::Refused,
        &helpers,
    )?;
    constrained::inside_triangles(face, &built)
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{BTreeMap, BTreeSet},
        f64::consts::TAU,
    };

    use super::*;

    const SIDE: f64 = 100.0;
    const HOLES: usize = 8;
    const HOLE_POINTS: usize = 64;
    const HOLE_RADIUS: f64 = 4.0;

    struct Plate {
        mapped: Vec<PlanePoint<f64>>,
        outline: Outline,
        area: f64,
    }

    fn plate(lattice: bool, stagger: f64) -> Plate {
        let mut mapped = vec![
            PlanePoint::new(0.0, 0.0),
            PlanePoint::new(SIDE, 0.0),
            PlanePoint::new(SIDE, SIDE),
            PlanePoint::new(0.0, SIDE),
        ];
        let mut loops = vec![vec![0, 1, 2, 3]];
        let pitch = SIDE / HOLES as f64;
        let mut centres = Vec::new();
        let mut area = SIDE * SIDE;
        for row in 0..HOLES {
            for column in 0..HOLES {
                let centre = PlanePoint::new(
                    pitch * (column as f64 + 0.5),
                    pitch * (row as f64 + 0.5) + stagger * column as f64,
                );
                let mut hole = Vec::new();
                for step in 0..HOLE_POINTS {
                    let angle = -TAU * step as f64 / HOLE_POINTS as f64;
                    hole.push(mapped.len());
                    mapped.push(PlanePoint::new(
                        centre.x + HOLE_RADIUS * angle.cos(),
                        centre.y + HOLE_RADIUS * angle.sin(),
                    ));
                }
                area -= 0.5
                    * HOLE_POINTS as f64
                    * HOLE_RADIUS
                    * HOLE_RADIUS
                    * (TAU / HOLE_POINTS as f64).sin();
                loops.push(hole);
                centres.push(centre);
            }
        }
        if lattice {
            for x in 1..SIDE as usize {
                for y in 1..SIDE as usize {
                    let at = PlanePoint::new(x as f64, y as f64);
                    let clear = centres
                        .iter()
                        .all(|centre| (at.x - centre.x).hypot(at.y - centre.y) > HOLE_RADIUS + 0.5);
                    if clear {
                        mapped.push(at);
                    }
                }
            }
        }
        Plate {
            outline: constrained::outline(&mapped, &loops),
            mapped,
            area,
        }
    }

    fn area(mapped: &[PlanePoint<f64>], triangles: &[[usize; 3]]) -> f64 {
        triangles
            .iter()
            .map(|[a, b, c]| {
                let (a, b, c) = (mapped[*a], mapped[*b], mapped[*c]);
                0.5 * ((b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x))
            })
            .sum()
    }

    fn assert_closed_over(plate: &Plate, triangles: &[[usize; 3]]) {
        let mut directed = BTreeSet::new();
        let mut uses: BTreeMap<(usize, usize), usize> = BTreeMap::new();
        for [a, b, c] in triangles {
            for (from, to) in [(*a, *b), (*b, *c), (*c, *a)] {
                assert!(directed.insert((from, to)), "edge {from}-{to} used twice");
                *uses.entry((from.min(to), from.max(to))).or_default() += 1;
            }
        }
        let boundary: BTreeSet<(usize, usize)> = plate
            .outline
            .segments
            .iter()
            .map(|(from, to)| (*from.min(to), *from.max(to)))
            .collect();
        for (edge, count) in uses {
            let wanted = if boundary.contains(&edge) { 1 } else { 2 };
            assert_eq!(count, wanted, "edge {edge:?}");
        }
    }

    #[test]
    fn a_large_face_triangulated_in_pieces_is_covered_once_without_gaps() {
        let face = FaceId::from_index(0).unwrap();
        for (lattice, stagger) in [(false, 0.3), (true, 0.3), (false, 0.0), (true, 0.0)] {
            let plate = plate(lattice, stagger);

            let whole = constrained::whole(face, &plate.mapped, &plate.outline).unwrap();
            let alone = triangles(face, &plate.mapped, &plate.outline, 1).unwrap();
            let shared = triangles(face, &plate.mapped, &plate.outline, 4).unwrap();
            let strips = strips(&plate.mapped, &plate.outline);
            let kept: Vec<[usize; 3]> = (0..strips.len())
                .flat_map(|strip| {
                    piece(face, &plate.mapped, &plate.outline, &strips, strip)
                        .unwrap()
                        .triangles
                })
                .collect();
            let certified = kept.len();
            let straddling = kept
                .iter()
                .filter(|corners| {
                    let cores = corners.map(|corner| core_of(&strips, plate.mapped[corner].x));
                    cores.iter().any(|core| *core != cores[0])
                })
                .count();

            assert!(plate.mapped.len() >= SPLIT_POINTS);
            assert!(strips.len() >= 4, "{} strips", strips.len());
            assert!(
                certified * 10 >= whole.len() * 9,
                "{certified} of {} triangles made in pieces",
                whole.len()
            );
            assert!(straddling > 0);
            let alone = alone.unwrap();
            assert_eq!(shared, Some(alone.clone()));
            assert_eq!(alone.len(), whole.len());
            assert_closed_over(&plate, &alone);
            assert_closed_over(&plate, &whole);
            let (pieced, entire) = (area(&plate.mapped, &alone), area(&plate.mapped, &whole));
            assert!((pieced - plate.area).abs() < 1e-9 * plate.area, "{pieced}");
            assert!((entire - plate.area).abs() < 1e-9 * plate.area, "{entire}");
        }
    }

    #[test]
    fn only_a_large_face_holding_much_of_its_body_is_triangulated_in_pieces() {
        let face = FaceId::from_index(0).unwrap();
        let square =
            [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)].map(|(x, y)| PlanePoint::new(x, y));
        let outline = constrained::outline(&square, &[vec![0, 1, 2, 3]]);

        assert!(allowed(SPLIT_POINTS, SPLIT_POINTS * DOMINANT_SHARE));
        assert!(!allowed(SPLIT_POINTS, SPLIT_POINTS * DOMINANT_SHARE + 1));
        assert!(!allowed(SPLIT_POINTS - 1, SPLIT_POINTS));
        assert!(strips(&square, &outline).is_empty());
        assert_eq!(triangles(face, &square, &outline, 4), Ok(None));
    }
}
