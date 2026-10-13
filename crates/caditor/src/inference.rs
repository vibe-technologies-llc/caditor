use crate::{
    body_snap::{BodyPart, BodyTarget},
    drawing::Direction,
    snap::Target,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Inference {
    Horizontal,
    Vertical,
    Parallel,
    Perpendicular,
    Tangent,
    Coincident,
    OnCurve,
    Midpoint,
    Centre,
    Intersection,
    Quadrant,
    Tracks,
}

impl Inference {
    pub const ALL: [Self; 12] = [
        Self::Horizontal,
        Self::Vertical,
        Self::Parallel,
        Self::Perpendicular,
        Self::Tangent,
        Self::Coincident,
        Self::OnCurve,
        Self::Midpoint,
        Self::Centre,
        Self::Intersection,
        Self::Quadrant,
        Self::Tracks,
    ];

    pub const DIRECTIONS: [Self; 5] = [
        Self::Horizontal,
        Self::Vertical,
        Self::Parallel,
        Self::Perpendicular,
        Self::Tangent,
    ];

    pub const TARGETS: [Self; 6] = [
        Self::Coincident,
        Self::OnCurve,
        Self::Midpoint,
        Self::Centre,
        Self::Intersection,
        Self::Quadrant,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::Horizontal => "sketch.infer.horizontal",
            Self::Vertical => "sketch.infer.vertical",
            Self::Parallel => "sketch.infer.parallel",
            Self::Perpendicular => "sketch.infer.perpendicular",
            Self::Tangent => "sketch.infer.tangent",
            Self::Coincident => "sketch.infer.coincident",
            Self::OnCurve => "sketch.infer.on_curve",
            Self::Midpoint => "sketch.infer.midpoint",
            Self::Centre => "sketch.infer.centre",
            Self::Intersection => "sketch.infer.intersection",
            Self::Quadrant => "sketch.infer.quadrant",
            Self::Tracks => "sketch.infer.tracks",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Horizontal => "Horizontal",
            Self::Vertical => "Vertical",
            Self::Parallel => "Parallel",
            Self::Perpendicular => "Perpendicular",
            Self::Tangent => "Tangent",
            Self::Coincident => "Coincident with a point",
            Self::OnCurve => "On a curve",
            Self::Midpoint => "At a midpoint",
            Self::Centre => "At the centre of an outline",
            Self::Intersection => "At an intersection",
            Self::Quadrant => "At a quadrant or axis end",
            Self::Tracks => "In line with a point passed over",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Horizontal => "A line drawn nearly level becomes horizontal",
            Self::Vertical => "A line drawn nearly upright becomes vertical",
            Self::Parallel => "A line drawn nearly parallel to another becomes parallel to it",
            Self::Perpendicular => {
                "A line drawn nearly square to another becomes perpendicular to it"
            }
            Self::Tangent => {
                "A line leaving a circle or arc, or ending where it would touch one, becomes \
                 tangent to it"
            }
            Self::Coincident => "A point placed on an existing point joins it",
            Self::OnCurve => "A point placed on a line, circle, arc, spline or axis stays on it",
            Self::Midpoint => "A point placed at the middle of a line or arc is held there",
            Self::Centre => "A point placed at the centre of an outline or a round is held there",
            Self::Intersection => "A point placed where two curves cross is held there",
            Self::Quadrant => {
                "A point placed at a quadrant of a circle or arc, or at the end of an ellipse's \
                 axis, is held there"
            }
            Self::Tracks => {
                "A point placed level or upright with, or along a line through, a point the \
                 pointer passed over is held in line with it"
            }
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Inferences {
    disabled: u16,
}

impl Inferences {
    pub fn allows(self, kind: Inference) -> bool {
        self.disabled & Self::bit(kind) == 0
    }

    pub fn set(&mut self, kind: Inference, on: bool) {
        if on {
            self.disabled &= !Self::bit(kind);
        } else {
            self.disabled |= Self::bit(kind);
        }
    }

    pub fn allows_direction(self, direction: Direction) -> bool {
        self.allows(match direction {
            Direction::Horizontal => Inference::Horizontal,
            Direction::Vertical => Inference::Vertical,
            Direction::Parallel(_) => Inference::Parallel,
            Direction::Perpendicular(_) => Inference::Perpendicular,
            Direction::Tangent(_) => Inference::Tangent,
        })
    }

    pub fn allows_target(self, target: Target) -> bool {
        match target {
            Target::Pending(_) => true,
            Target::Point(_) => self.allows(Inference::Coincident),
            Target::Curve(_) | Target::Extension(_) => self.allows(Inference::OnCurve),
            Target::Midpoint(_) => self.allows(Inference::Midpoint),
            Target::Quadrant { .. } | Target::AxisEnd { .. } => self.allows(Inference::Quadrant),
            Target::Tangent(_) => self.allows(Inference::Tangent),
            Target::Intersection(..) => self.allows(Inference::Intersection),
            Target::Centre { .. } | Target::Centroid(_) => self.allows(Inference::Centre),
            Target::Body(target) => self.allows_body(target),
        }
    }

    pub fn allows_body(self, target: BodyTarget) -> bool {
        self.allows_part(target.part)
    }

    pub fn allows_part(self, part: BodyPart) -> bool {
        self.allows(match part {
            BodyPart::Corner => Inference::Coincident,
            BodyPart::Middle => Inference::Midpoint,
            BodyPart::Centre => Inference::Centre,
            BodyPart::Edge => Inference::OnCurve,
        })
    }

    fn bit(kind: Inference) -> u16 {
        1 << kind as u16
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_starts_on_and_turns_off_alone() {
        let mut inferences = Inferences::default();

        assert!(
            Inference::ALL
                .into_iter()
                .all(|kind| inferences.allows(kind))
        );

        inferences.set(Inference::Midpoint, false);

        for kind in Inference::ALL {
            assert_eq!(
                inferences.allows(kind),
                kind != Inference::Midpoint,
                "{kind:?}"
            );
        }

        inferences.set(Inference::Midpoint, true);

        assert_eq!(inferences, Inferences::default());
    }

    #[test]
    fn the_keys_are_distinct_and_the_groups_cover_every_kind() {
        let keys: Vec<&str> = Inference::ALL.iter().map(|kind| kind.key()).collect();
        for (index, key) in keys.iter().enumerate() {
            assert!(!keys[index + 1..].contains(key), "{key} is used twice");
        }

        let grouped: Vec<Inference> = Inference::DIRECTIONS
            .into_iter()
            .chain(Inference::TARGETS)
            .chain([Inference::Tracks])
            .collect();

        assert_eq!(grouped, Inference::ALL);
    }
}
