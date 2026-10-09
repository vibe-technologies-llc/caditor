use std::collections::BTreeSet;

use caditor_geometry::{Aabb2, Point2};
use thiserror::Error;

use crate::{
    build::curves,
    curve2::Curve2,
    error::GeometryError,
    interrupt::{self, Interrupted},
    profile::{
        PieceBound, PieceId, ProfileCurve, ProfileError, ProfileLoop, Region, RegionKey,
        source::Source,
        strand::{OffsetFailure, Strand, offset_strands, polygon, polygons_cross, signed_area},
    },
    tolerance::{LINEAR_RESOLUTION, MAX_SIZE},
};

const RELATIVE_TOLERANCE: f64 = 1e-7;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum WallSide {
    Inside,
    Outside,
    Centred,
}

#[derive(Debug, Clone, PartialEq, Error)]
pub enum WallError {
    #[error("the sketch has no curves to give a wall")]
    NoCurves,
    #[error("the wall thickness is not a finite number")]
    NonFinite,
    #[error("the wall needs a thickness greater than zero")]
    NotPositive,
    #[error("the wall is thicker than {MAX_SIZE} mm")]
    BeyondMaximum,
    #[error("{} cannot be given a wall; only lines, arcs and circles can", curves(.entities))]
    UnsupportedCurve { entities: Vec<u64> },
    #[error("three or more curves meet at one point: {}", curves(.entities))]
    Branches { entities: Vec<u64> },
    #[error("the curves turn straight back where {} meet", curves(.entities))]
    Folds { entities: Vec<u64> },
    #[error("the wall is too thick for {}", curves(.entities))]
    TooThick { entities: Vec<u64> },
    #[error("the sides of the wall do not meet where {} meet", curves(.entities))]
    Apart { entities: Vec<u64> },
    #[error("the wall runs into itself")]
    CrossesItself,
    #[error(transparent)]
    Curve(#[from] ProfileError),
    #[error("the wall cannot be built: {0}")]
    Geometry(#[from] GeometryError),
    #[error(transparent)]
    Cancelled(#[from] Interrupted),
}

impl WallError {
    pub fn entities(&self) -> Vec<u64> {
        match self {
            Self::UnsupportedCurve { entities }
            | Self::Branches { entities }
            | Self::Folds { entities }
            | Self::TooThick { entities }
            | Self::Apart { entities } => entities.clone(),
            Self::NoCurves
            | Self::NonFinite
            | Self::NotPositive
            | Self::BeyondMaximum
            | Self::CrossesItself
            | Self::Curve(_)
            | Self::Geometry(_)
            | Self::Cancelled(_) => Vec::new(),
        }
    }
}

struct Element {
    entity: u64,
    strand: Strand,
}

#[derive(Debug, Clone, Copy)]
struct Link {
    element: usize,
    forward: bool,
}

struct Chain {
    links: Vec<Link>,
    closed: bool,
}

fn own_right(entity: u64) -> PieceId {
    PieceId::new(entity, PieceBound::Start, PieceBound::End)
}

fn own_left(entity: u64) -> PieceId {
    PieceId::new(entity, PieceBound::End, PieceBound::Start)
}

fn own_start(entity: u64) -> PieceId {
    PieceId::new(entity, PieceBound::Start, PieceBound::Start)
}

fn own_end(entity: u64) -> PieceId {
    PieceId::new(entity, PieceBound::End, PieceBound::End)
}

pub fn wall_regions(
    curves: &[ProfileCurve],
    thickness: f64,
    side: WallSide,
) -> Result<Vec<Region>, WallError> {
    if !thickness.is_finite() {
        return Err(WallError::NonFinite);
    }
    if thickness <= LINEAR_RESOLUTION {
        return Err(WallError::NotPositive);
    }
    if thickness > MAX_SIZE {
        return Err(WallError::BeyondMaximum);
    }
    if curves.is_empty() {
        return Err(WallError::NoCurves);
    }
    let sources: Vec<Source> = curves
        .iter()
        .map(Source::from_curve)
        .collect::<Result<_, _>>()?;
    let unsupported: Vec<u64> = sources
        .iter()
        .filter(|source| matches!(source.curve, Curve2::BSpline(_) | Curve2::Ellipse(_)))
        .map(|source| source.entity)
        .collect();
    if !unsupported.is_empty() {
        return Err(WallError::UnsupportedCurve {
            entities: unsupported,
        });
    }
    let elements: Vec<Element> = sources
        .iter()
        .filter_map(|source| {
            Some(Element {
                entity: source.entity,
                strand: Strand::of_curve(&source.curve, source.range, false)?,
            })
        })
        .collect();
    let size = sources
        .iter()
        .map(|source| source.curve.bounding_box(source.range))
        .reduce(Aabb2::union)
        .map_or(1.0, |bounds| {
            bounds.size().length() + bounds.min().abs().max(bounds.max().abs()).max_element()
        });
    let tolerance = (size * RELATIVE_TOLERANCE).max(LINEAR_RESOLUTION);
    let chains = chains(&elements, tolerance)?;
    let mut regions = Vec::with_capacity(chains.len());
    for chain in &chains {
        interrupt::check()?;
        regions.push(chain_region(&elements, chain, thickness, side, tolerance)?);
    }
    let polygons: Vec<Vec<Point2>> = regions
        .iter()
        .flat_map(Region::loops)
        .map(|profile_loop| {
            let strands: Vec<Strand> = profile_loop
                .pieces()
                .iter()
                .filter_map(Strand::of_piece)
                .collect();
            polygon(&strands)
        })
        .collect();
    if polygons_cross(&polygons) {
        return Err(WallError::CrossesItself);
    }
    Ok(regions)
}

fn chains(elements: &[Element], tolerance: f64) -> Result<Vec<Chain>, WallError> {
    let ends: Vec<(usize, bool, Point2)> = elements
        .iter()
        .enumerate()
        .filter(|(_, element)| !element.strand.is_full_circle())
        .flat_map(|(index, element)| {
            [
                (index, true, element.strand.start()),
                (index, false, element.strand.end()),
            ]
        })
        .collect();
    let mut partner: Vec<Option<usize>> = vec![None; ends.len()];
    for (position, (element, _, point)) in ends.iter().enumerate() {
        let touching: Vec<usize> = ends
            .iter()
            .enumerate()
            .filter(|(other, (_, _, at))| *other != position && at.distance(*point) <= tolerance)
            .map(|(other, _)| other)
            .collect();
        match touching.as_slice() {
            [] => {}
            [only] => {
                if let Some(slot) = partner.get_mut(position) {
                    *slot = Some(*only);
                }
            }
            several => {
                let mut entities: BTreeSet<u64> = several
                    .iter()
                    .filter_map(|other| ends.get(*other))
                    .filter_map(|(other, _, _)| elements.get(*other))
                    .map(|element| element.entity)
                    .collect();
                entities.extend(elements.get(*element).map(|element| element.entity));
                return Err(WallError::Branches {
                    entities: entities.into_iter().collect(),
                });
            }
        }
    }
    let end_of = |element: usize, at_start: bool| {
        ends.iter()
            .position(|(other, start, _)| *other == element && *start == at_start)
    };
    let mut taken = vec![false; elements.len()];
    let mut chains = Vec::new();
    for (index, element) in elements.iter().enumerate() {
        if element.strand.is_full_circle() {
            if let Some(slot) = taken.get_mut(index) {
                *slot = true;
            }
            chains.push(Chain {
                links: vec![Link {
                    element: index,
                    forward: true,
                }],
                closed: true,
            });
        }
    }
    let open_starts: Vec<(usize, bool)> = ends
        .iter()
        .zip(&partner)
        .filter(|(_, partner)| partner.is_none())
        .map(|((element, at_start, _), _)| (*element, *at_start))
        .collect();
    let starts = open_starts
        .into_iter()
        .map(|start| (start, false))
        .chain((0..elements.len()).map(|element| ((element, true), true)));
    for ((first, at_start), closed) in starts {
        if taken.get(first).copied().unwrap_or(true) {
            continue;
        }
        let mut links = Vec::new();
        let (mut element, mut entering_start) = (first, at_start);
        loop {
            if let Some(slot) = taken.get_mut(element) {
                *slot = true;
            }
            links.push(Link {
                element,
                forward: entering_start,
            });
            let next = end_of(element, !entering_start)
                .and_then(|exit| partner.get(exit).copied().flatten())
                .and_then(|other| ends.get(other));
            match next {
                Some((other, other_start, _)) if !taken.get(*other).copied().unwrap_or(true) => {
                    element = *other;
                    entering_start = *other_start;
                }
                Some(_) | None => break,
            }
        }
        chains.push(Chain { links, closed });
    }
    Ok(chains)
}

fn chain_region(
    elements: &[Element],
    chain: &Chain,
    thickness: f64,
    side: WallSide,
    tolerance: f64,
) -> Result<Region, WallError> {
    let linked: Vec<(&Element, bool)> = chain
        .links
        .iter()
        .filter_map(|link| Some((elements.get(link.element)?, link.forward)))
        .collect();
    let strands: Vec<Strand> = linked
        .iter()
        .map(|(element, forward)| {
            if *forward {
                element.strand
            } else {
                element.strand.reversed()
            }
        })
        .collect();
    let entities: Vec<u64> = linked.iter().map(|(element, _)| element.entity).collect();
    let inside_is_left = signed_area(&polygon(&strands)) >= 0.0;
    let (left, right) = match (side, inside_is_left) {
        (WallSide::Centred, _) => (0.5 * thickness, 0.5 * thickness),
        (WallSide::Inside, true) | (WallSide::Outside, false) => (thickness, 0.0),
        (WallSide::Inside, false) | (WallSide::Outside, true) => (0.0, thickness),
    };
    let failure = |failure: OffsetFailure| {
        let named: Vec<u64> = failure
            .strands()
            .into_iter()
            .filter_map(|index| entities.get(index).copied())
            .collect();
        match failure {
            OffsetFailure::Shrinks(_) => WallError::TooThick { entities: named },
            OffsetFailure::Apart(..) => WallError::Apart { entities: named },
            OffsetFailure::Folds(..) => WallError::Folds { entities: named },
        }
    };
    let left_side =
        offset_strands(&strands, chain.closed, left, None, tolerance).map_err(failure)?;
    let right_side =
        offset_strands(&strands, chain.closed, -right, None, tolerance).map_err(failure)?;
    let right_pieces = right_side
        .iter()
        .zip(&linked)
        .map(|(strand, (element, forward))| {
            let id = if *forward {
                own_right(element.entity)
            } else {
                own_left(element.entity)
            };
            strand.piece(id)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let left_pieces = left_side
        .iter()
        .zip(&linked)
        .rev()
        .map(|(strand, (element, forward))| {
            let id = if *forward {
                own_left(element.entity)
            } else {
                own_right(element.entity)
            };
            strand.reversed().piece(id)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let right_loop = ProfileLoop {
        pieces: right_pieces,
    };
    let left_loop = ProfileLoop {
        pieces: left_pieces,
    };
    let (outer, holes) = if chain.closed {
        if right_loop.signed_area() > 0.0 {
            (right_loop, vec![left_loop])
        } else {
            (left_loop, vec![right_loop])
        }
    } else {
        let (Some((first, first_forward)), Some((last, last_forward))) =
            (linked.first(), linked.last())
        else {
            return Err(WallError::NoCurves);
        };
        let (Some(right_first), Some(right_last), Some(left_first), Some(left_last)) = (
            right_side.first(),
            right_side.last(),
            left_side.first(),
            left_side.last(),
        ) else {
            return Err(WallError::NoCurves);
        };
        let end_cap = Strand::Line {
            start: right_last.end(),
            end: left_last.end(),
        }
        .piece(if *last_forward {
            own_end(last.entity)
        } else {
            own_start(last.entity)
        })?;
        let start_cap = Strand::Line {
            start: left_first.start(),
            end: right_first.start(),
        }
        .piece(if *first_forward {
            own_start(first.entity)
        } else {
            own_end(first.entity)
        })?;
        let mut pieces = right_loop.pieces;
        pieces.push(end_cap);
        pieces.extend(left_loop.pieces);
        pieces.push(start_cap);
        (ProfileLoop { pieces }, Vec::new())
    };
    let hole_area: f64 = holes.iter().map(ProfileLoop::signed_area).sum();
    if outer.signed_area() <= tolerance * tolerance || hole_area > 0.0 {
        return Err(WallError::CrossesItself);
    }
    let key = RegionKey::of_sides(
        outer
            .pieces
            .iter()
            .chain(holes.iter().flat_map(|hole| &hole.pieces)),
    );
    Ok(Region {
        key,
        depth: 0,
        outer,
        holes,
    })
}
