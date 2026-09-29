use caditor_geometry::Ray;
use caditor_kernel::{Curve, Edge, EdgeId, FaceId, Sense, Solid};

use crate::tolerance;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unresolved {
    Missing(usize),
    Unrelated(usize),
}

pub(crate) struct Tally<T> {
    found: Vec<T>,
    missing: usize,
    unrelated: usize,
}

impl<T: Ord> Tally<T> {
    pub(crate) fn with_capacity(capacity: usize) -> Self {
        Self {
            found: Vec::with_capacity(capacity),
            missing: 0,
            unrelated: 0,
        }
    }

    pub(crate) fn found(&mut self, item: T) {
        self.found.push(item);
    }

    pub(crate) fn pieces(&mut self, pieces: Vec<T>, related: bool) {
        if related {
            self.found.extend(pieces);
        } else {
            self.unrelated += 1;
        }
    }

    pub(crate) fn missing(&mut self) {
        self.missing += 1;
    }

    pub(crate) fn finish(mut self) -> Result<Vec<T>, Unresolved> {
        if self.missing > 0 {
            return Err(Unresolved::Missing(self.missing));
        }
        if self.unrelated > 0 {
            return Err(Unresolved::Unrelated(self.unrelated));
        }
        self.found.sort_unstable();
        self.found.dedup();
        Ok(self.found)
    }
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
