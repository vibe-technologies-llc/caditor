use std::collections::{BTreeSet, VecDeque};

use spade::{
    Point2 as PlanePoint, Triangulation,
    handles::{FixedFaceHandle, InnerTag},
};

use crate::{
    interrupt::{self, Interrupted},
    tessellation::{
        POLL_EVERY, TessellationError,
        constrained::{self, Built, Splitting},
        parallel,
    },
    topology::FaceId,
};

pub(super) const SPLIT_POINTS: usize = 4096;
const POINTS_PER_PIECE: usize = 1024;
const MAX_PIECES: usize = 16;
const REACH_SHARE: usize = 4;
const DOMINANT_SHARE: usize = 4;

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
}

impl Strip {
    fn owns(&self, x: f64) -> bool {
        self.core.low <= x && x < self.core.high
    }
}

#[derive(Debug, Default)]
struct Piece {
    triangles: Vec<[usize; 3]>,
    border: Vec<(usize, usize)>,
    covered: Vec<(usize, usize)>,
}

pub(super) fn allowed(points: usize, body_points: usize) -> bool {
    points >= SPLIT_POINTS && points.saturating_mul(DOMINANT_SHARE) >= body_points
}

pub(super) fn triangles(
    face: FaceId,
    mapped: &[PlanePoint<f64>],
    segments: &[(usize, usize)],
    threads: usize,
) -> Result<Option<Vec<[usize; 3]>>, Interrupted> {
    let strips = strips(mapped);
    if strips.len() < 2 {
        return Ok(None);
    }
    match in_pieces(face, mapped, segments, &strips, threads) {
        Ok(triangles) => Ok(Some(triangles)),
        Err(TessellationError::Cancelled(interrupted)) => Err(interrupted),
        Err(_) => Ok(None),
    }
}

fn strips(mapped: &[PlanePoint<f64>]) -> Vec<Strip> {
    let count = mapped.len();
    let pieces = (count / POINTS_PER_PIECE).min(MAX_PIECES);
    if count < SPLIT_POINTS || pieces < 2 {
        return Vec::new();
    }
    let mut xs: Vec<f64> = mapped.iter().map(|point| point.x).collect();
    xs.sort_by(f64::total_cmp);
    let at = |rank: usize| xs.get(rank.min(count - 1)).copied();
    let reach = count / pieces / REACH_SHARE;
    let mut cuts: Vec<(f64, usize)> = vec![(f64::NEG_INFINITY, 0)];
    for piece in 1..pieces {
        let rank = piece * count / pieces;
        let (Some(before), Some(after)) = (at(rank.saturating_sub(1)), at(rank)) else {
            return Vec::new();
        };
        let cut = 0.5 * (before + after);
        if cut.is_finite() && cuts.last().is_some_and(|(last, _)| *last < cut) {
            cuts.push((cut, rank));
        }
    }
    cuts.push((f64::INFINITY, count));
    cuts.windows(2)
        .filter_map(|pair| match pair {
            [(low, first), (high, end)] => Some(Strip {
                core: Span {
                    low: *low,
                    high: *high,
                },
                reach: Span {
                    low: if low.is_finite() {
                        at(first.saturating_sub(reach))?.min(*low)
                    } else {
                        f64::NEG_INFINITY
                    },
                    high: if high.is_finite() {
                        at(end + reach)?.max(*high)
                    } else {
                        f64::INFINITY
                    },
                },
            }),
            _ => None,
        })
        .collect()
}

fn in_pieces(
    face: FaceId,
    mapped: &[PlanePoint<f64>],
    segments: &[(usize, usize)],
    strips: &[Strip],
    threads: usize,
) -> Result<Vec<[usize; 3]>, TessellationError> {
    let pieces = parallel::each_in_order(strips.len(), threads, 1, |index| {
        let strip = strips
            .get(index)
            .ok_or(TessellationError::Triangulation(face))?;
        piece(face, mapped, segments, strip)
    });
    let pieces = pieces.into_iter().collect::<Result<Vec<Piece>, _>>()?;
    let remainder = remainder(face, mapped, segments, &pieces)?;
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

fn piece(
    face: FaceId,
    mapped: &[PlanePoint<f64>],
    segments: &[(usize, usize)],
    strip: &Strip,
) -> Result<Piece, TessellationError> {
    interrupt::check()?;
    let mut member: Vec<bool> = mapped.iter().map(|at| strip.reach.holds(at.x)).collect();
    let mut crossing = Vec::new();
    for (index, (from, to)) in segments.iter().enumerate() {
        if index.is_multiple_of(POLL_EVERY) {
            interrupt::check()?;
        }
        let (a, b) = (point(mapped, *from, face)?, point(mapped, *to, face)?);
        if strip.reach.overlaps(a.x, b.x) {
            crossing.push((*from, *to));
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
    let built = constrained::build(face, mapped, members, &crossing, Splitting::Allowed)?;
    let x = |vertex: usize| {
        built
            .members
            .get(vertex)
            .and_then(|member| mapped.get(*member))
            .map(|at| at.x)
    };
    let within = |fixed: FixedFaceHandle<InnerTag>| {
        built
            .cdt
            .face(fixed)
            .vertices()
            .iter()
            .all(|vertex| x(vertex.fix().index()).is_some_and(|x| strip.reach.holds(x)))
    };
    let inside = inside_within(&built, mapped, &crossing, face, &within)?;
    let mut certified = vec![false; built.cdt.num_all_faces()];
    for fixed in built.cdt.fixed_inner_faces() {
        if let Some(slot) = certified.get_mut(fixed.index()) {
            *slot = inside.get(fixed.index()).copied().unwrap_or(false)
                && certifies(&built, mapped, strip, fixed);
        }
    }
    let is_certified =
        |fixed: FixedFaceHandle<InnerTag>| certified.get(fixed.index()).copied().unwrap_or(false);
    let mut made = Piece::default();
    for (step, face_handle) in built.cdt.inner_faces().enumerate() {
        if step.is_multiple_of(POLL_EVERY) {
            interrupt::check()?;
        }
        if !is_certified(face_handle.fix()) {
            continue;
        }
        made.triangles.push(
            built
                .corners(face_handle.fix())
                .ok_or(TessellationError::Triangulation(face))?,
        );
        for edge in face_handle.adjacent_edges() {
            let [from, to] = [edge.from(), edge.to()]
                .map(|vertex| built.members.get(vertex.fix().index()).copied());
            let (Some(from), Some(to)) = (from, to) else {
                return Err(TessellationError::Triangulation(face));
            };
            if built.cdt.is_constraint_edge(edge.fix().as_undirected()) {
                made.covered.push((from.min(to), from.max(to)));
            } else if !edge
                .rev()
                .face()
                .as_inner()
                .is_some_and(|neighbour| is_certified(neighbour.fix()))
            {
                made.border.push((from, to));
            }
        }
    }
    Ok(made)
}

fn certifies(
    built: &Built,
    mapped: &[PlanePoint<f64>],
    strip: &Strip,
    fixed: FixedFaceHandle<InnerTag>,
) -> bool {
    let Some(corners) = built.corners(fixed) else {
        return false;
    };
    let [Some(a), Some(b), Some(c)] = corners.map(|corner| mapped.get(corner).copied()) else {
        return false;
    };
    if ![a, b, c].iter().all(|at| strip.owns(at.x)) {
        return false;
    }
    let (bx, by, cx, cy) = (b.x - a.x, b.y - a.y, c.x - a.x, c.y - a.y);
    let twice = 2.0 * (bx * cy - by * cx);
    let (b_square, c_square) = (bx * bx + by * by, cx * cx + cy * cy);
    let centre_x = (cy * b_square - by * c_square) / twice;
    let centre_y = (bx * c_square - cx * b_square) / twice;
    let radius = centre_x.hypot(centre_y);
    let centre = a.x + centre_x;
    radius.is_finite() && strip.reach.low < centre - radius && centre + radius < strip.reach.high
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
        let corners = built
            .corners(fixed)
            .ok_or(TessellationError::Triangulation(face))?;
        let mut centroid = PlanePoint::new(0.0, 0.0);
        for corner in corners {
            let at = point(mapped, corner, face)?;
            centroid.x += at.x / 3.0;
            centroid.y += at.y / 3.0;
        }
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
    segments: &[(usize, usize)],
    pieces: &[Piece],
) -> Result<Vec<[usize; 3]>, TessellationError> {
    interrupt::check()?;
    let covered: BTreeSet<(usize, usize)> = pieces
        .iter()
        .flat_map(|piece| piece.covered.iter().copied())
        .collect();
    let mut used = vec![false; mapped.len()];
    for corner in pieces
        .iter()
        .flat_map(|piece| piece.triangles.iter().flatten())
    {
        if let Some(slot) = used.get_mut(*corner) {
            *slot = true;
        }
    }
    let mut constraints: Vec<(usize, usize)> = pieces
        .iter()
        .flat_map(|piece| piece.border.iter().copied())
        .collect();
    constraints.extend(
        segments
            .iter()
            .filter(|(from, to)| !covered.contains(&(*from.min(to), *from.max(to)))),
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
    let built = constrained::build(face, mapped, members, &constraints, Splitting::Refused)?;
    constrained::inside_triangles(face, &built)
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, f64::consts::TAU};

    use super::*;

    const SIDE: f64 = 100.0;
    const HOLES: usize = 8;
    const HOLE_POINTS: usize = 64;
    const HOLE_RADIUS: f64 = 4.0;

    struct Plate {
        mapped: Vec<PlanePoint<f64>>,
        segments: Vec<(usize, usize)>,
        area: f64,
    }

    fn plate(lattice: bool) -> Plate {
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
                    pitch * (row as f64 + 0.5) + 0.3 * column as f64,
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
            mapped,
            segments: constrained::segments(&loops),
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
        for lattice in [false, true] {
            let plate = plate(lattice);

            let whole = constrained::whole(face, &plate.mapped, &plate.segments).unwrap();
            let alone = triangles(face, &plate.mapped, &plate.segments, 1).unwrap();
            let shared = triangles(face, &plate.mapped, &plate.segments, 4).unwrap();
            let strips = strips(&plate.mapped);
            let certified: usize = strips
                .iter()
                .map(|strip| {
                    piece(face, &plate.mapped, &plate.segments, strip)
                        .unwrap()
                        .triangles
                        .len()
                })
                .sum();

            assert!(plate.mapped.len() >= SPLIT_POINTS);
            assert!(strips.len() >= 4, "{} strips", strips.len());
            assert!(
                certified * 3 >= whole.len() * 2,
                "{certified} of {} triangles made in pieces",
                whole.len()
            );
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
        let segments = constrained::segments(&[vec![0, 1, 2, 3]]);

        assert!(allowed(SPLIT_POINTS, SPLIT_POINTS * DOMINANT_SHARE));
        assert!(!allowed(SPLIT_POINTS, SPLIT_POINTS * DOMINANT_SHARE + 1));
        assert!(!allowed(SPLIT_POINTS - 1, SPLIT_POINTS));
        assert!(strips(&square).is_empty());
        assert_eq!(triangles(face, &square, &segments, 4), Ok(None));
    }
}
