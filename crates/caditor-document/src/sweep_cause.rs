use caditor_expression::format_decimals;
use caditor_kernel::PipeShape;

use crate::{failure::Cause, pipe::wall_limit_words, sweep::capital};

const TWIST_DECIMALS: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Follower {
    Feature,
    Sweep,
    Pipe,
    Shaped,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Segments {
    pub labels: String,
    pub owner: String,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum SweepCause {
    PathUnavailable {
        sketch: String,
    },
    CurveGone {
        sketch: String,
        feature: String,
    },
    EdgeAmbiguous {
        body: String,
        feature: String,
    },
    EdgeGone {
        body: String,
        feature: String,
    },
    EdgeMissing {
        body: String,
    },
    HelixUnavailable {
        helix: String,
    },
    EndScale {
        feature: String,
        scale: f64,
    },
    ShortSegment {
        label: String,
        owner: String,
        follower: Follower,
    },
    Branches {
        at: Segments,
    },
    Apart {
        at: Segments,
        single: bool,
    },
    Unsupported {
        name: String,
        label: String,
        owner: String,
    },
    TooLong,
    Intricate {
        label: String,
        owner: String,
        follower: Follower,
    },
    Corner {
        at: Segments,
        name: String,
        follower: Follower,
    },
    CornerTooSharp {
        at: Segments,
        name: String,
        follower: Follower,
    },
    ProfileTooLarge {
        name: String,
        label: String,
        owner: String,
        sketch: Option<String>,
    },
    ClosedTwists,
    CrossesItself {
        name: String,
        follower: Follower,
    },
    Unbuilt {
        name: String,
    },
    StartsAlongProfile {
        sketch: String,
    },
    TurnsAlongProfile {
        label: String,
        owner: String,
    },
    ClosedMissesProfile {
        sketch: String,
    },
    ClosedTwistsBy {
        name: String,
        turned: f64,
        needed: f64,
    },
    ClosedTilted,
    ClosedScales {
        name: String,
    },
    UnusableShape {
        name: String,
    },
    WallTooThick {
        name: String,
        shape: PipeShape,
    },
    SpiralTurnsTouch {
        name: String,
        spiral: String,
        pitch: f64,
        least: f64,
    },
    BendTooLong {
        at: Segments,
    },
    BendTooTight {
        at: Segments,
        name: String,
    },
    PipeUnbuilt {
        name: String,
    },
}

impl SweepCause {
    pub(crate) fn reason(&self) -> String {
        match self {
            Self::PathUnavailable { sketch } => {
                format!("It needs the path drawn in {sketch}, which is not available.")
            }
            Self::CurveGone { sketch, feature } => format!(
                "A curve of the path is no longer a curve of {sketch}, so {feature} cannot \
                 follow it."
            ),
            Self::EdgeAmbiguous { body, feature } => format!(
                "An edge of the path now matches several edges of {body}, so {feature} cannot \
                 tell which to follow."
            ),
            Self::EdgeGone { body, feature } => format!(
                "An edge of the path is no longer part of {body}, so {feature} cannot follow it."
            ),
            Self::EdgeMissing { body } => {
                format!("An edge of the path is no longer part of {body}.")
            }
            Self::HelixUnavailable { helix } => {
                format!("It needs {helix} as its path, which is not available.")
            }
            Self::EndScale { feature, scale } => format!(
                "The end scale of {feature} must be more than zero, and {} is not.",
                format_decimals(*scale, TWIST_DECIMALS)
            ),
            Self::ShortSegment {
                label,
                owner,
                follower,
            } => {
                let along = match follower {
                    Follower::Sweep => "sweep along",
                    _ => "follow",
                };
                format!("{}{owner} is too short to {along}.", capital(label))
            }
            Self::Branches { at } => {
                format!("The path branches where {}{} meet.", at.labels, at.owner)
            }
            Self::Apart { at, single } => format!(
                "{}{} {} not join the rest of the path.",
                capital(&at.labels),
                at.owner,
                if *single { "does" } else { "do" }
            ),
            Self::Unsupported { name, label, owner } => {
                format!("{name} cannot follow {label}{owner}.")
            }
            Self::TooLong => "The path reaches farther than the largest model allows.".to_owned(),
            Self::Intricate {
                label,
                owner,
                follower,
            } => {
                let along = match follower {
                    Follower::Sweep => "sweep along",
                    _ => "follow",
                };
                format!(
                    "{}{owner} bends too intricately to {along}.",
                    capital(label)
                )
            }
            Self::Corner { at, name, follower } => {
                let which = match follower {
                    Follower::Feature => format!("{name} cannot follow"),
                    Follower::Sweep => "a profile kept square to the path cannot follow".to_owned(),
                    Follower::Pipe => format!("{name} cannot follow without a bend"),
                    Follower::Shaped => "a twisted or scaled sweep cannot mitre".to_owned(),
                };
                format!(
                    "The path turns a sharp corner between {}{}, which {which}.",
                    at.labels, at.owner
                )
            }
            Self::CornerTooSharp { at, name, follower } => match follower {
                Follower::Sweep => format!(
                    "The corner between {}{} is too sharp for the profile: its mitre would cut \
                     past the end of a line.",
                    at.labels, at.owner
                ),
                Follower::Pipe => format!(
                    "The corner between {}{} is too sharp for {name} to turn.",
                    at.labels, at.owner
                ),
                Follower::Feature | Follower::Shaped => format!(
                    "The corner between {}{} turns back too sharply for {name} to follow.",
                    at.labels, at.owner
                ),
            },
            Self::ProfileTooLarge {
                name,
                label,
                owner,
                sketch,
            } => match sketch {
                Some(sketch) => format!(
                    "The profile of {sketch} is too large for the bend of {label}{owner}: it \
                     would reach past the centre the path bends around."
                ),
                None => format!(
                    "{name} is too large for the bend of {label}{owner}: it would reach past the \
                     centre the path bends around."
                ),
            },
            Self::ClosedTwists => {
                "Following the closed path brings the section back turned, so it would not close."
                    .to_owned()
            }
            Self::CrossesItself { name, .. } => format!("{name} runs into itself along its path."),
            Self::Unbuilt { name } => format!("{name} cannot be built along this path."),
            Self::StartsAlongProfile { sketch } => format!(
                "The path leaves the profile of {sketch} along the profile's own plane, so the \
                 sweep would be flat."
            ),
            Self::TurnsAlongProfile { label, owner } => format!(
                "Kept parallel to its sketch, the profile would slide along itself where \
                 {label}{owner} turns along the profile's plane."
            ),
            Self::ClosedMissesProfile { sketch } => format!(
                "The closed path never crosses the plane of {sketch}, so the profile is not on it."
            ),
            Self::ClosedTwistsBy { name, turned, .. } => format!(
                "Following the closed path brings the profile back turned by {}°, so {name} \
                 would not close.",
                format_decimals(*turned, TWIST_DECIMALS)
            ),
            Self::ClosedTilted => {
                "Following the closed path brings the profile back tilted, so the sweep would \
                 not close."
                    .to_owned()
            }
            Self::ClosedScales { name } => format!(
                "{name} follows a closed path, so its end is its start and cannot be scaled."
            ),
            Self::UnusableShape { name } => {
                format!("The twist or end scale of {name} is out of range.")
            }
            Self::WallTooThick { name, shape } => format!(
                "The wall of {name} is too thick: it must be thinner than {}.",
                wall_limit_words(*shape)
            ),
            Self::SpiralTurnsTouch {
                name,
                spiral,
                pitch,
                least,
            } => format!(
                "The turns of {name} would touch along {spiral}: at a pitch of {pitch:.3} mm its \
                 section needs more than {least:.3} mm between turns."
            ),
            Self::BendTooLong { at } => format!(
                "Rounding the corner between {}{} to the bend radius needs more than the length \
                 of the curves there.",
                at.labels, at.owner
            ),
            Self::BendTooTight { at, name } => format!(
                "The bend between {}{} is tighter than {name}: its section would reach past the \
                 centre the bend turns around.",
                at.labels, at.owner
            ),
            Self::PipeUnbuilt { name } => format!("{name} cannot be built with these sizes."),
        }
    }

    pub(crate) fn remedy(&self) -> String {
        match self {
            Self::PathUnavailable { sketch } => {
                format!("Fix {sketch} first, or choose another path.")
            }
            Self::CurveGone { .. }
            | Self::EdgeAmbiguous { .. }
            | Self::EdgeGone { .. }
            | Self::EdgeMissing { .. } => "Choose the path again.".to_owned(),
            Self::HelixUnavailable { helix } => {
                format!("Fix {helix} first, or choose another path.")
            }
            Self::EndScale { .. } => {
                "Enter an end scale above zero, such as 0.5 to end at half the size or 1 to keep \
                 it."
                .to_owned()
            }
            Self::ShortSegment { .. } => "Lengthen it, or leave it out of the path.".to_owned(),
            Self::Branches { .. } => {
                "Choose curves or edges that follow one another end to end, without forks."
                    .to_owned()
            }
            Self::Apart { .. } => {
                "Choose curves or edges that meet end to end, or close the gap between them."
                    .to_owned()
            }
            Self::Unsupported { .. } => {
                "Choose lines, arcs, splines or edges as the path.".to_owned()
            }
            Self::TooLong => "Choose a shorter path.".to_owned(),
            Self::Intricate { .. } => "Simplify that curve.".to_owned(),
            Self::Corner { follower, .. } => match follower {
                Follower::Feature => {
                    "Round the corner with a fillet in the path, or meet it with straight lines \
                     on both sides."
                        .to_owned()
                }
                Follower::Sweep => {
                    "Round the corner with a fillet in the path, keep the profile parallel, or \
                     meet it with straight lines on both sides so it is mitred."
                        .to_owned()
                }
                Follower::Pipe => {
                    "Give the pipe a bend radius, or round the corner with a fillet in the \
                     path's sketch."
                        .to_owned()
                }
                Follower::Shaped => {
                    "Round the corner with a fillet in the path, or set the twist to 0° and the \
                     end scale to 1."
                        .to_owned()
                }
            },
            Self::CornerTooSharp { follower, .. } => match follower {
                Follower::Sweep => {
                    "Lengthen the lines at the corner, open the corner up, or make the profile \
                     smaller."
                        .to_owned()
                }
                Follower::Pipe => {
                    "Open the corner up, lengthen the lines at it, or give the pipe a bend \
                     radius."
                        .to_owned()
                }
                Follower::Feature | Follower::Shaped => {
                    "Open the corner up, or lengthen the lines at it.".to_owned()
                }
            },
            Self::ProfileTooLarge { sketch, .. } => match sketch {
                Some(_) => "Make the profile smaller or the bend gentler.".to_owned(),
                None => "Make it smaller or the bend gentler.".to_owned(),
            },
            Self::ClosedTwists => {
                "Use a closed path lying in one plane, or an open path.".to_owned()
            }
            Self::CrossesItself { follower, .. } => match follower {
                Follower::Sweep => {
                    "Make the profile smaller, or move the parts of the path farther apart."
                        .to_owned()
                }
                _ => "Make it smaller, or move the parts of the path farther apart.".to_owned(),
            },
            Self::Unbuilt { .. } => "Choose another path or change its sizes.".to_owned(),
            Self::StartsAlongProfile { .. } => {
                "Draw the path so it starts across the profile's plane, in a sketch on a plane \
                 crossing the profile's."
                    .to_owned()
            }
            Self::TurnsAlongProfile { .. } => {
                "Keep the profile square to the path, or choose a path that keeps rising out of \
                 the profile's plane."
                    .to_owned()
            }
            Self::ClosedMissesProfile { .. } => {
                "Move the profile onto the path, or the path through the profile.".to_owned()
            }
            Self::ClosedTwistsBy { needed, .. } => format!(
                "Set the twist to {}°, or that plus or minus whole turns, so the profile comes \
                 back where it started.",
                format_decimals(*needed, TWIST_DECIMALS)
            ),
            Self::ClosedTilted => {
                "Draw the profile square to the path where they meet, use a closed path lying in \
                 one plane, or an open path."
                    .to_owned()
            }
            Self::ClosedScales { .. } => {
                "Set the end scale to 1, or choose an open path.".to_owned()
            }
            Self::UnusableShape { .. } => "Enter an end scale nearer 1.".to_owned(),
            Self::WallTooThick { .. } => "Make the wall thinner or the pipe larger.".to_owned(),
            Self::SpiralTurnsTouch { name, spiral, .. } => {
                format!("Make the pitch of {spiral} larger, or {name} smaller.")
            }
            Self::BendTooLong { .. } => {
                "Make the bend radius smaller, or lengthen the curves at the corner.".to_owned()
            }
            Self::BendTooTight { .. } => {
                "Make the bend radius larger or the pipe smaller.".to_owned()
            }
            Self::PipeUnbuilt { .. } => {
                "Change its size, wall thickness or bend radius.".to_owned()
            }
        }
    }
}

impl From<SweepCause> for Cause {
    fn from(cause: SweepCause) -> Self {
        Self::Sweep(cause)
    }
}
