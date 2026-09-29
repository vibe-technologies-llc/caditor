use caditor_geometry::Ray;
use caditor_kernel::{Curve, Edge, EdgeId, FaceId, ReferenceError, Sense, Solid};

use crate::tolerance;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unresolved {
    Missing(usize),
    Unrelated(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution<T> {
    One(T),
    Pieces(Vec<T>),
    Tied(Vec<T>),
    Missing,
}

impl<T> Resolution<T> {
    pub(crate) fn of(
        resolved: Result<T, ReferenceError<T>>,
        related: impl FnOnce(&[T]) -> bool,
    ) -> Self {
        match resolved {
            Ok(item) => Self::One(item),
            Err(ReferenceError::Ambiguous(pieces)) if related(&pieces) => Self::Pieces(pieces),
            Err(ReferenceError::Ambiguous(pieces)) => Self::Tied(pieces),
            Err(ReferenceError::Missing) => Self::Missing,
        }
    }

    pub fn found(&self) -> &[T] {
        match self {
            Self::One(item) => std::slice::from_ref(item),
            Self::Pieces(items) | Self::Tied(items) => items,
            Self::Missing => &[],
        }
    }
}

pub(crate) fn tally<T: Ord>(resolutions: Vec<Resolution<T>>) -> Result<Vec<T>, Unresolved> {
    let mut found = Vec::with_capacity(resolutions.len());
    let mut missing = 0;
    let mut unrelated = 0_usize;
    for resolution in resolutions {
        match resolution {
            Resolution::One(item) => found.push(item),
            Resolution::Pieces(pieces) => found.extend(pieces),
            Resolution::Tied(_) => unrelated += 1,
            Resolution::Missing => missing += 1,
        }
    }
    if missing > 0 {
        return Err(Unresolved::Missing(missing));
    }
    if unrelated > 0 {
        return Err(Unresolved::Unrelated(unrelated));
    }
    found.sort_unstable();
    found.dedup();
    Ok(found)
}

pub(crate) fn pieces_of_one_edge(solid: &Solid, pieces: &[EdgeId]) -> bool {
    let curves: Vec<&Curve> = pieces
        .iter()
        .filter_map(|id| solid.edge(*id))
        .map(Edge::curve)
        .collect();
    let Some((first, rest)) = curves.split_first() else {
        return false;
    };
    curves.len() == pieces.len() && rest.iter().all(|other| one_curve(first, other))
}

fn one_curve(first: &Curve, other: &Curve) -> bool {
    match (first, other) {
        (Curve::Line(first), Curve::Line(other)) => {
            match (
                Ray::new(first.origin(), first.direction()),
                Ray::new(other.origin(), other.direction()),
            ) {
                (Some(first), Some(other)) => tolerance::same_line(first, other),
                _ => false,
            }
        }
        (Curve::Circle(first), Curve::Circle(other)) => {
            first.center().distance(other.center()) <= tolerance::POSITION_TOLERANCE
                && (first.radius() - other.radius()).abs() <= tolerance::POSITION_TOLERANCE
                && tolerance::parallel(first.frame().normal(), other.frame().normal())
        }
        _ => first == other,
    }
}

pub(crate) fn pieces_of_one_face(solid: &Solid, pieces: &[FaceId]) -> bool {
    let faces: Vec<_> = pieces.iter().filter_map(|id| solid.face(*id)).collect();
    let Some((first, rest)) = faces.split_first() else {
        return false;
    };
    faces.len() == pieces.len()
        && rest.iter().all(|other| {
            first
                .surface()
                .same_surface(other.surface())
                .is_some_and(|sense| {
                    sense.combined(first.sense()).combined(other.sense()) == Sense::Same
                })
        })
}
