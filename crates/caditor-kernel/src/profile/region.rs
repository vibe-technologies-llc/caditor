use std::collections::{BTreeMap, BTreeSet};

use crate::profile::{
    Piece, ProfileLoop, Region, RegionKey,
    arrangement::{Arrangement, UnionFind, piece_of},
};

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Draft {
    outer: Vec<Piece>,
    holes: Vec<Vec<Piece>>,
    depth: usize,
    faces: BTreeSet<usize>,
}

fn piece(arrangement: &Arrangement, half_edge: usize) -> Option<Piece> {
    let (index, forward) = piece_of(half_edge);
    let graph = arrangement.pieces.get(index)?;
    Some(Piece {
        id: graph.id.clone(),
        curve: graph.curve.clone(),
        range: graph.range,
        reversed: !forward,
    })
}

pub(super) fn lumps(arrangement: &Arrangement, chosen: &BTreeSet<usize>) -> Vec<Draft> {
    let is_chosen = |half_edge: usize| {
        arrangement
            .face_of(half_edge)
            .is_some_and(|face| chosen.contains(&face))
    };
    let is_kept = |half_edge: usize| is_chosen(half_edge) && !is_chosen(half_edge ^ 1);
    let starts: BTreeSet<usize> = chosen
        .iter()
        .flat_map(|face| arrangement.half_edges_of(*face).iter().copied())
        .collect();
    let Ok(cycles) = arrangement.cycles_from(starts.iter().copied(), is_kept) else {
        return Vec::new();
    };
    let area = |cycle: &[usize]| -> f64 {
        cycle
            .iter()
            .map(|half_edge| arrangement.half_edge_area(*half_edge))
            .sum()
    };
    let mut lump_of = joined_faces(arrangement, chosen, &starts);
    let (outers, holes): (Vec<&Vec<usize>>, Vec<&Vec<usize>>) =
        cycles.iter().partition(|cycle| area(cycle) > 0.0);
    let mut outers_of_lump: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (index, outer) in outers.iter().enumerate() {
        if let Some(lump) = lump_of(outer) {
            outers_of_lump.entry(lump).or_default().push(index);
        }
    }
    let every_outer: Vec<usize> = (0..outers.len()).collect();
    let mut assigned: Vec<Vec<&Vec<usize>>> = vec![Vec::new(); outers.len()];
    for hole in holes {
        let lump_outers = lump_of(hole).and_then(|lump| outers_of_lump.get(&lump));
        let container = match lump_outers.map(Vec::as_slice) {
            Some([only]) => Some(*only),
            _ => {
                let candidates = lump_outers.unwrap_or(&every_outer);
                let Some(probe) = hole
                    .first()
                    .and_then(|half_edge| arrangement.origin(*half_edge).ok())
                    .and_then(|vertex| arrangement.vertices.get(vertex).copied())
                else {
                    continue;
                };
                candidates
                    .iter()
                    .filter_map(|index| Some((*index, *outers.get(*index)?)))
                    .filter(|(_, outer)| arrangement.contains(outer, probe))
                    .min_by(|a, b| area(a.1).total_cmp(&area(b.1)))
                    .map(|(index, _)| index)
            }
        };
        if let Some(list) = container.and_then(|index| assigned.get_mut(index)) {
            list.push(hole);
        }
    }
    outers
        .into_iter()
        .zip(assigned)
        .map(|(outer, holes)| {
            let faces: BTreeSet<usize> = std::iter::once(outer)
                .chain(holes.iter().copied())
                .flatten()
                .filter_map(|half_edge| {
                    let (index, forward) = piece_of(*half_edge);
                    let [left, right] = arrangement.sides.get(index)?;
                    if forward { *left } else { *right }
                })
                .collect();
            let depth = faces
                .iter()
                .filter_map(|face| arrangement.faces.get(*face))
                .map(|face| face.depth)
                .min()
                .unwrap_or(0);
            let pieces = |cycle: &Vec<usize>| -> Vec<Piece> {
                cycle
                    .iter()
                    .filter_map(|half_edge| piece(arrangement, *half_edge))
                    .collect()
            };
            Draft {
                outer: pieces(outer),
                holes: holes.into_iter().map(pieces).collect(),
                depth,
                faces,
            }
        })
        .collect()
}

fn joined_faces<'a>(
    arrangement: &'a Arrangement,
    chosen: &BTreeSet<usize>,
    half_edges: &BTreeSet<usize>,
) -> impl FnMut(&[usize]) -> Option<usize> + 'a {
    let members: Vec<usize> = chosen.iter().copied().collect();
    let member = move |face: usize| members.binary_search(&face).ok();
    let mut sets = UnionFind::new(chosen.len());
    for half_edge in half_edges {
        let across = arrangement
            .face_of(*half_edge)
            .and_then(&member)
            .zip(arrangement.face_of(half_edge ^ 1).and_then(&member));
        if let Some((face, beyond)) = across {
            sets.union(face, beyond);
        }
    }
    move |cycle: &[usize]| {
        let face = arrangement.face_of(*cycle.first()?)?;
        Some(sets.find(member(face)?))
    }
}

fn all_pieces(draft: &Draft) -> impl Iterator<Item = &Piece> {
    draft.outer.iter().chain(draft.holes.iter().flatten())
}

pub(super) fn base_keys(drafts: &[Draft]) -> Vec<RegionKey> {
    drafts
        .iter()
        .map(|draft| RegionKey::of_sides(all_pieces(draft)))
        .collect()
}

fn counts(bases: &[RegionKey]) -> BTreeMap<RegionKey, usize> {
    let mut counts = BTreeMap::new();
    for base in bases {
        *counts.entry(*base).or_insert(0) += 1;
    }
    counts
}

pub(super) fn shared_keys(bases: &[RegionKey]) -> BTreeSet<RegionKey> {
    counts(bases)
        .into_iter()
        .filter(|(_, count)| *count > 1)
        .map(|(base, _)| base)
        .collect()
}

pub(super) fn with_keys(
    drafts: Vec<Draft>,
    ambiguous: &BTreeSet<RegionKey>,
) -> Vec<(Region, BTreeSet<usize>)> {
    let bases = base_keys(&drafts);
    let counts = counts(&bases);
    let mut keyed: Vec<(Region, BTreeSet<usize>)> = drafts
        .into_iter()
        .zip(&bases)
        .map(|(draft, base)| {
            let shared =
                ambiguous.contains(base) || counts.get(base).is_some_and(|count| *count > 1);
            let key = if shared {
                base.tiebroken(all_pieces(&draft))
            } else {
                *base
            };
            let region = Region {
                key,
                depth: draft.depth,
                outer: ProfileLoop {
                    pieces: draft.outer,
                },
                holes: draft
                    .holes
                    .into_iter()
                    .map(|pieces| ProfileLoop { pieces })
                    .collect(),
            };
            (region, draft.faces)
        })
        .collect();
    keyed.sort_by_key(|(region, _)| region.key);
    keyed
}
