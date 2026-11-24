use caditor_geometry::Point2;

use crate::{
    curve2::{Curve2, Line2},
    interval::Interval,
    profile::{Piece, PieceBound, PieceId, ProfileLoop, Region},
};

const PARAMETER_SLACK: f64 = 1e-9;

pub(super) fn joined(region: Region, tolerance: f64) -> Region {
    let joints: Vec<Point2> = region.pieces().map(Piece::start).collect();
    let rejoined = |profile_loop: ProfileLoop| ProfileLoop {
        pieces: joined_loop(profile_loop.pieces, &joints, tolerance),
    };

    Region {
        key: region.key,
        depth: region.depth,
        outer: rejoined(region.outer),
        holes: region.holes.into_iter().map(rejoined).collect(),
    }
}

fn joined_loop(pieces: Vec<Piece>, joints: &[Point2], tolerance: f64) -> Vec<Piece> {
    let mut kept: Vec<Piece> = Vec::with_capacity(pieces.len());

    for piece in pieces {
        let longer = kept
            .last()
            .and_then(|last| continued(last, &piece, joints, tolerance));

        match (longer, kept.last_mut()) {
            (Some(longer), Some(last)) => *last = longer,
            _ => kept.push(piece),
        }
    }

    let across_the_start = match (kept.as_slice(), kept.last(), kept.first()) {
        ([_, _, ..], Some(last), Some(first)) => continued(last, first, joints, tolerance),
        _ => None,
    };

    if let Some(longer) = across_the_start {
        kept.pop();
        if let Some(first) = kept.first_mut() {
            *first = longer;
        }
    }
    if let [only] = kept.as_mut_slice() {
        whole_turn(only);
    }
    kept
}

fn is_shared(joints: &[Point2], joint: Point2, tolerance: f64) -> bool {
    joints
        .iter()
        .filter(|other| other.distance(joint) <= tolerance)
        .nth(1)
        .is_some()
}

fn continued(first: &Piece, second: &Piece, joints: &[Point2], tolerance: f64) -> Option<Piece> {
    if first.entity() != second.entity()
        || first.reversed != second.reversed
        || is_shared(joints, second.start(), tolerance)
    {
        return None;
    }

    let (before, after) = if first.reversed {
        (second, first)
    } else {
        (first, second)
    };

    let id = PieceId::new(
        first.entity(),
        before.id.start.clone(),
        after.id.end.clone(),
    );

    let (curve, range) = match (&before.curve, &after.curve) {
        (Curve2::Line(near), Curve2::Line(far)) => {
            if near.direction().dot(far.direction()) <= 0.0 {
                return None;
            }

            let from = before.curve.point(before.range.start());
            let to = after.curve.point(after.range.end());

            let line = Line2::through(from, to).ok()?;

            (Curve2::from(line), Interval::new(0.0, from.distance(to))?)
        }
        (carrier, other) if carrier == other => {
            if !runs_on(carrier, before.range, after.range) {
                return None;
            }
            (
                carrier.clone(),
                Interval::new(
                    before.range.start(),
                    before.range.end() + after.range.length(),
                )?,
            )
        }
        _ => return None,
    };

    Some(Piece {
        id,
        curve,
        range,
        reversed: first.reversed,
    })
}

fn runs_on(carrier: &Curve2, before: Interval, after: Interval) -> bool {
    let step = after.start() - before.end();
    let gap = match carrier.period() {
        Some(period) => step - (step / period).round() * period,
        None => step,
    };

    gap.abs() <= PARAMETER_SLACK
}

fn whole_turn(piece: &mut Piece) {
    let Some(period) = piece.curve.period() else {
        return;
    };

    if piece.range.length() < period - PARAMETER_SLACK {
        return;
    }
    piece.id = PieceId::new(piece.entity(), PieceBound::Start, PieceBound::End);
    piece.range = Interval::FULL_TURN;
}
