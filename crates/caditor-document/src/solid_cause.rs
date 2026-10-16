use caditor_expression::Quantity;
use caditor_kernel::{MAX_SIZE, MAX_TAPER_DEGREES};

use crate::{
    attachment::AttachmentError,
    failure::Cause,
    solid::{AxisSide, Side},
    trouble::Trouble,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TargetFor {
    Start { shape: &'static str },
    ExtrusionEnd,
    RevolutionEnd,
}

impl TargetFor {
    fn subject(self) -> String {
        match self {
            Self::Start { shape } => format!("this {shape} starts from"),
            Self::ExtrusionEnd => "this extrusion runs up to".to_owned(),
            Self::RevolutionEnd => "this revolution turns up to".to_owned(),
        }
    }

    fn remedy(self) -> &'static str {
        match self {
            Self::Start { .. } => {
                "Select a flat face or plane and use it as the start, or enter a distance."
            }
            Self::ExtrusionEnd => {
                "Select a flat face or plane and use it for this end, or choose another end."
            }
            Self::RevolutionEnd => {
                "Select a flat face or plane through the axis and use it for the end, or enter an \
                 angle."
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CurveProblem {
    NoLength,
    BeyondModel,
    DoublesBack,
    Alongside,
    TooCrossed,
    Undivided,
    Unusable,
}

impl CurveProblem {
    fn words(self) -> &'static str {
        match self {
            Self::NoLength => "has no length",
            Self::BeyondModel => "reaches farther than caditor models",
            Self::DoublesBack => "runs back over itself",
            Self::Alongside => "run along each other",
            Self::TooCrossed => "cross too often to be divided into regions",
            Self::Undivided => "could not be divided into closed regions",
            Self::Unusable => "cannot be used",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Gap {
    pub first: String,
    pub second: String,
    pub distance: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SquareEnd {
    UpToNext,
    UpToCurved,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum SolidCause {
    TaperWithDirection {
        feature: String,
    },
    WallWithSide,
    OnlyRemovalCutsSeveral,
    Target {
        error: AttachmentError,
        body: String,
        target: TargetFor,
    },
    StartNotParallel {
        plane: String,
        sketch: String,
        shape: &'static str,
    },
    StartUnplaceable {
        shape: &'static str,
        sketch: String,
    },
    OffsetUnapplicable {
        sketch: String,
    },
    NotPositive {
        what: String,
    },
    SteepTaper {
        feature: String,
    },
    WallNoCurves {
        sketch: String,
    },
    WallNotPositive,
    WallTooBig,
    WallBranches {
        sketch: String,
        curves: String,
    },
    WallFolds {
        sketch: String,
        curves: String,
    },
    WallTooThick {
        feature: String,
        sketch: String,
        curves: String,
    },
    WallApart {
        feature: String,
        sketch: String,
        curves: String,
    },
    WallCrosses {
        feature: String,
        sketch: String,
    },
    WallUnbuilt {
        sketch: String,
    },
    NoClosedShape {
        sketch: String,
        gaps: Vec<Gap>,
        unnamed: usize,
    },
    RegionGone {
        sketch: String,
    },
    RegionAmbiguous {
        sketch: String,
        candidates: usize,
    },
    CurvesUndivided {
        sketch: String,
    },
    CurvesProblem {
        sketch: String,
        curves: String,
        problem: CurveProblem,
        several: bool,
    },
    NoLength {
        shape: String,
    },
    TooLong {
        shape: String,
    },
    NoAngle,
    TurnsTwice,
    DirectionAlongSketch {
        shape: String,
        sketch: String,
    },
    EndAlongDirection,
    EndsCross {
        sketch: String,
    },
    AxisWithoutDirection,
    BothSides {
        sketch: String,
        curves: String,
        several: bool,
    },
    CrossesAxis {
        sketch: String,
        curves: String,
    },
    OnAxis {
        sketch: String,
    },
    RegionUnbuilt {
        shape: String,
        sketch: String,
        curves: String,
        several: bool,
    },
    TaperUnsupported {
        sketch: String,
        curves: String,
    },
    TaperTilted {
        shape: String,
        sketch: String,
        feature: String,
    },
    TaperCloses {
        feature: String,
        sketch: String,
        at: Option<String>,
        shape: String,
    },
    Unbuilt {
        shape: String,
        sketch: String,
    },
    Vanishes {
        body: String,
        intersecting: bool,
    },
    Edgewise {
        body: String,
    },
    Uncombined {
        body: String,
        trouble: Trouble,
    },
    ExtrusionEmpty,
    TwoSidedStop {
        sketch: String,
    },
    SurfaceFace {
        body: String,
        split: bool,
    },
    EndNotMoved {
        name: String,
        offset: &'static str,
    },
    EndOffsetShort {
        offset: f64,
        name: String,
        label: &'static str,
    },
    ThroughAllAdds,
    NothingToGoThrough {
        body: String,
        side: Side,
    },
    NextNeedsBody,
    NextUnoffsettable {
        reached: Box<Cause>,
    },
    AddInside {
        sketch: String,
        body: String,
    },
    RemoveEntering {
        sketch: String,
        body: String,
    },
    ReachNothing {
        sketch: String,
        body: String,
        side: Side,
    },
    ReachPartly {
        sketch: String,
        body: String,
    },
    ReachSeveral {
        sketch: String,
        body: String,
        faces: Vec<String>,
    },
    ReachCurved {
        face: String,
    },
    ReachUndecided {
        body: String,
        side: Side,
    },
    SquareOnly {
        end: SquareEnd,
    },
    EndAlong {
        name: String,
    },
    EndNotAhead {
        name: String,
        side: Side,
    },
    EndCuts {
        name: String,
        sketch: String,
    },
    StopStraddles {
        sketch: String,
        body: String,
    },
    StopEnclosed {
        body: String,
    },
    StopUnbuilt {
        body: String,
    },
    StopBeforeFace {
        sketch: String,
        others: Vec<String>,
        body: String,
        face: String,
    },
    NotThroughAxis {
        name: String,
    },
    WouldNotTurn {
        name: String,
        sketch: String,
    },
    AnglesOverflow,
    DirectionAlong {
        axis: String,
        sketch: String,
    },
    AxisOffPlane {
        axis: String,
        sketch: String,
    },
    NoSideLeft {
        sketch: String,
        side: AxisSide,
    },
    NotALine {
        axis: String,
        sketch: String,
    },
}

impl SolidCause {
    pub(crate) fn reason(&self) -> String {
        let most = MAX_SIZE / 1_000.0;
        match self {
            Self::TaperWithDirection { feature } => format!(
                "A tapered extrusion runs square to its sketch, and {feature} follows a \
                 direction."
            ),
            Self::WallWithSide => {
                "A thin wall follows the whole of its curves, so it cannot keep one side of the \
                 revolution axis."
                    .to_owned()
            }
            Self::OnlyRemovalCutsSeveral => {
                "Only a removal cuts several bodies at once.".to_owned()
            }
            Self::Target {
                error,
                body,
                target,
            } => {
                let subject = target.subject();
                match error {
                    AttachmentError::Missing => {
                        format!("The face {subject} is no longer part of the body of {body}.")
                    }
                    AttachmentError::Ambiguous => format!(
                        "The face of {body} {subject} was split into parts that no longer lie in \
                         one plane."
                    ),
                    AttachmentError::NotFlat => {
                        format!("The face of {body} {subject} is no longer flat.")
                    }
                }
            }
            Self::StartNotParallel {
                plane,
                sketch,
                shape,
            } => format!(
                "{plane} is not parallel to the plane of {sketch}, so the {shape} cannot start \
                 from it."
            ),
            Self::StartUnplaceable { shape, sketch } => {
                format!("The {shape} cannot start from the plane of {sketch}.")
            }
            Self::OffsetUnapplicable { sketch } => {
                format!("The start offset cannot be applied to the plane of {sketch}.")
            }
            Self::NotPositive { what } => format!("The {what} must be more than zero."),
            Self::SteepTaper { feature } => {
                format!("The taper of {feature} is {MAX_TAPER_DEGREES}° or steeper.")
            }
            Self::WallNoCurves { sketch } => {
                format!("{sketch} has no curves for the thin wall to follow.")
            }
            Self::WallNotPositive => "The wall thickness must be more than zero.".to_owned(),
            Self::WallTooBig => format!("The wall thickness cannot be more than {most} m."),
            Self::WallBranches { sketch, curves } => format!(
                "In {sketch}, {curves} meet at one point, so the thin wall cannot tell which way \
                 to follow."
            ),
            Self::WallFolds { sketch, curves } => {
                format!("In {sketch}, {curves} turn straight back on each other.")
            }
            Self::WallTooThick {
                feature,
                sketch,
                curves,
            } => format!("The wall of {feature} is too thick for {curves} of {sketch}."),
            Self::WallApart {
                feature, curves, ..
            } => format!("The sides of the wall of {feature} do not meet where {curves} meet."),
            Self::WallCrosses { feature, .. } => {
                format!("The wall of {feature} runs into itself.")
            }
            Self::WallUnbuilt { sketch } => {
                format!("The thin wall of {sketch} could not be built.")
            }
            Self::NoClosedShape {
                sketch,
                gaps,
                unnamed,
            } => match Self::gaps_text(gaps, *unnamed) {
                Some(gaps) => format!("{sketch} has no closed shape to sweep: {gaps}."),
                None => format!("{sketch} has no closed shape to sweep."),
            },
            Self::RegionGone { sketch } => {
                format!("A chosen region of {sketch} no longer exists, because the sketch changed.")
            }
            Self::RegionAmbiguous { sketch, candidates } => format!(
                "A chosen region of {sketch} was divided, and {candidates} of the new regions \
                 match it equally."
            ),
            Self::CurvesUndivided { sketch } => {
                format!("The curves of {sketch} could not be divided into closed regions.")
            }
            Self::CurvesProblem {
                sketch,
                curves,
                problem,
                ..
            } => format!("In {sketch}, {curves} {}.", problem.words()),
            Self::NoLength { shape } => format!("The {shape} has no length."),
            Self::TooLong { shape } => format!("The {shape} reaches farther than {most} m."),
            Self::NoAngle => "The revolution has no angle.".to_owned(),
            Self::TurnsTwice => "The revolution turns more than once.".to_owned(),
            Self::DirectionAlongSketch { shape, sketch } => {
                format!("The direction of the {shape} runs along the plane of {sketch}.")
            }
            Self::EndAlongDirection => {
                "An end of the extrusion runs along its direction, so the extrusion never reaches \
                 it."
                .to_owned()
            }
            Self::EndsCross { sketch } => format!(
                "The two ends of the extrusion meet or cross within the profile of {sketch}."
            ),
            Self::AxisWithoutDirection => "The revolution axis has no direction.".to_owned(),
            Self::BothSides {
                sketch,
                curves,
                several,
            } => {
                let verb = if *several { "lie" } else { "lies" };
                format!(
                    "In {sketch}, {curves} {verb} on the other side of the revolution axis from \
                     the rest of the profile."
                )
            }
            Self::CrossesAxis { sketch, curves } => {
                format!("In {sketch}, {curves} would cut through the revolution axis.")
            }
            Self::OnAxis { sketch } => {
                format!("The profile of {sketch} lies on the revolution axis.")
            }
            Self::RegionUnbuilt {
                shape,
                sketch,
                curves,
                ..
            } => format!(
                "The {shape} of the region of {sketch} bounded by {curves} could not be built."
            ),
            Self::TaperUnsupported { sketch, curves } => format!(
                "In {sketch}, {curves} cannot be tapered; a taper follows only lines, arcs and \
                 circles."
            ),
            Self::TaperTilted {
                shape,
                sketch,
                feature,
            } => format!(
                "A tapered {shape} must end on faces or planes parallel to {sketch}, and an end \
                 of {feature} is slanted."
            ),
            Self::TaperCloses {
                feature,
                sketch,
                at,
                shape,
            } => {
                let at = at
                    .as_ref()
                    .map(|curves| format!(" at {curves}"))
                    .unwrap_or_default();
                format!(
                    "The taper of {feature} closes the profile of {sketch}{at} before the \
                     {shape} ends."
                )
            }
            Self::Unbuilt { shape, sketch } => {
                format!("The {shape} of {sketch} could not be built.")
            }
            Self::Vanishes {
                body,
                intersecting: true,
            } => {
                format!("The shape does not overlap the body of {body}, so nothing would be left.")
            }
            Self::Vanishes { body, .. } => format!("This removes all of the body of {body}."),
            Self::Edgewise { body } => {
                format!("The body of {body} would be left with parts that meet only along an edge.")
            }
            Self::Uncombined { body, trouble } => trouble.reason(format!(
                "The shape could not be combined with the body of {body}."
            )),
            Self::ExtrusionEmpty => "The extrusion has no length.".to_owned(),
            Self::TwoSidedStop { sketch } => format!(
                "The profile of {sketch} first meets curved or several faces, and an extrusion to \
                 two sides can only stop at one flat face."
            ),
            Self::SurfaceFace { body, split: false } => format!(
                "The face this extrusion runs up to is no longer part of the body of {body}."
            ),
            Self::SurfaceFace { body, split: true } => format!(
                "The face of {body} this extrusion runs up to was split, and several parts match \
                 it equally."
            ),
            Self::EndNotMoved { name, .. } => {
                format!("The end cannot be moved from {name} by its offset.")
            }
            Self::EndOffsetShort { offset, name, .. } => format!(
                "Stopping {} short of {name} would end the extrusion before it starts in places.",
                Quantity::length(-offset)
            ),
            Self::ThroughAllAdds => {
                "Through all only cuts into a body or intersects with it, so it cannot add \
                 material."
                    .to_owned()
            }
            Self::NothingToGoThrough { body, side } => format!(
                "No part of the body of {body} lies {}, so there is nothing to go through.",
                side.ahead()
            ),
            Self::NextNeedsBody => {
                "Up to next stops at the body this extrusion changes, and it makes a new body \
                 instead."
                    .to_owned()
            }
            Self::NextUnoffsettable { reached } => {
                format!("{} So the end cannot be offset from it.", reached.reason())
            }
            Self::AddInside { sketch, body } => format!(
                "The profile of {sketch} starts inside the body of {body}, so extruding it up to \
                 the next face adds nothing."
            ),
            Self::RemoveEntering { sketch, body } => format!(
                "The profile of {sketch} first meets the body of {body} where it enters it, so \
                 extruding up to that face removes nothing."
            ),
            Self::ReachNothing { sketch, body, side } => format!(
                "The profile of {sketch} meets nothing of the body of {body} {}.",
                side.ahead()
            ),
            Self::ReachPartly { sketch, body } => format!(
                "Part of the profile of {sketch} passes beside the body of {body}, so there is no \
                 one next face to stop at."
            ),
            Self::ReachSeveral {
                sketch,
                body,
                faces,
            } => format!(
                "The profile of {sketch} meets several faces of {body} first: {}.",
                crate::document::list_names(faces)
            ),
            Self::ReachCurved { face } => format!(
                "The next face, {face}, is curved, and an extrusion can only end on a flat face \
                 or plane."
            ),
            Self::ReachUndecided { body, side } => format!(
                "The next face of the body of {body} {} could not be found.",
                side.ahead()
            ),
            Self::SquareOnly { end } => {
                let end = match end {
                    SquareEnd::UpToNext => "Up to next",
                    SquareEnd::UpToCurved => "An end up to a curved face",
                };
                format!("{end} runs square to the sketch, so it cannot follow a direction.")
            }
            Self::EndAlong { name } => format!(
                "{} runs along the direction of the extrusion, so the extrusion never reaches it.",
                crate::datum::capitalized(name)
            ),
            Self::EndNotAhead { name, side } => format!(
                "{} does not lie {}.",
                crate::datum::capitalized(name),
                side.ahead()
            ),
            Self::EndCuts { name, sketch } => format!(
                "{} cuts across the profile of {sketch}, so the extrusion would end before it \
                 starts in places.",
                crate::datum::capitalized(name)
            ),
            Self::StopStraddles { sketch, body } => format!(
                "The profile of {sketch} starts partly inside the body of {body} and partly \
                 outside it, so there is no one next face to stop at."
            ),
            Self::StopEnclosed { body } => format!(
                "The body of {body} lies wholly within the extrusion, so it has no next face to \
                 stop at."
            ),
            Self::StopUnbuilt { body } => {
                format!("The extrusion could not be stopped where it meets the body of {body}.")
            }
            Self::StopBeforeFace {
                sketch,
                others,
                body,
                face,
            } => format!(
                "The profile of {sketch} meets {} of {body} before it reaches {face}.",
                crate::document::list_names(others)
            ),
            Self::NotThroughAxis { name } => format!(
                "{name} does not hold the revolution axis, so the revolution cannot end on it."
            ),
            Self::WouldNotTurn { name, sketch } => {
                format!("{name} holds the profile of {sketch}, so the revolution would not turn.")
            }
            Self::AnglesOverflow => "The two angles add up to more than 360°.".to_owned(),
            Self::DirectionAlong { axis, sketch } => format!(
                "{axis} runs along the plane of {sketch}, so the extrusion cannot follow it."
            ),
            Self::AxisOffPlane { axis, sketch } => format!(
                "{axis} does not lie in the plane of {sketch}, so the sketch cannot turn about it."
            ),
            Self::NoSideLeft { sketch, side } => format!(
                "No part of the profile of {sketch} lies on the {} side of the revolution axis.",
                side.name()
            ),
            Self::NotALine { axis, sketch } => {
                format!("The revolution axis, {axis}, is not a line of {sketch}.")
            }
        }
    }

    pub(crate) fn remedy(&self) -> String {
        match self {
            Self::TaperWithDirection { .. } => {
                "Set the taper to 0°, or run the extrusion square to the sketch.".to_owned()
            }
            Self::WallWithSide => "Turn the whole profile, or turn off the thin wall.".to_owned(),
            Self::OnlyRemovalCutsSeveral => {
                "Choose Remove from body, or leave out the other bodies.".to_owned()
            }
            Self::Target { target, .. } => target.remedy().to_owned(),
            Self::StartNotParallel { .. } => {
                "Choose a face or plane parallel to the sketch, or enter a distance.".to_owned()
            }
            Self::StartUnplaceable { .. } => "Choose another face or plane.".to_owned(),
            Self::OffsetUnapplicable { .. } => "Enter another start offset.".to_owned(),
            Self::NotPositive { what } => {
                format!("Enter a {what} above zero, or flip the direction instead.")
            }
            Self::SteepTaper { .. } => format!(
                "Enter a taper angle between -{MAX_TAPER_DEGREES}° and {MAX_TAPER_DEGREES}°."
            ),
            Self::WallNoCurves { sketch } => {
                format!("Draw the lines and arcs of the wall in {sketch}.")
            }
            Self::WallNotPositive => "Enter a wall thickness above zero.".to_owned(),
            Self::WallTooBig => "Enter a thinner wall.".to_owned(),
            Self::WallBranches { sketch, .. } => format!(
                "Leave only two curves meeting at each point in {sketch}, or make the others \
                 construction geometry."
            ),
            Self::WallFolds { sketch, .. } => format!("Open up that corner in {sketch}."),
            Self::WallTooThick { .. } => {
                "Enter a thinner wall, or place it on the other side of the curves.".to_owned()
            }
            Self::WallApart { sketch, .. } => {
                format!("Enter a thinner wall, or change that corner in {sketch}.")
            }
            Self::WallCrosses { sketch, .. } => format!(
                "Enter a thinner wall, place it on the other side, or move the curves of \
                 {sketch} apart."
            ),
            Self::WallUnbuilt { sketch } => {
                format!("Undo the last change, or simplify the curves of {sketch}.")
            }
            Self::NoClosedShape { sketch, .. } => format!(
                "Close the outline in {sketch}, for example by joining the ends of its lines."
            ),
            Self::RegionGone { .. } | Self::RegionAmbiguous { .. } => {
                "Choose the regions again.".to_owned()
            }
            Self::CurvesUndivided { sketch } => {
                format!("Simplify where curves meet or cross in {sketch}.")
            }
            Self::CurvesProblem {
                sketch, several, ..
            } => {
                let which = if *several { "one of them" } else { "it" };
                format!("Change or delete {which} in {sketch}.")
            }
            Self::NoLength { .. } => "Enter a distance other than zero.".to_owned(),
            Self::TooLong { .. } => "Enter a shorter distance.".to_owned(),
            Self::NoAngle => "Enter an angle other than zero.".to_owned(),
            Self::TurnsTwice => "Enter an angle of at most 360°.".to_owned(),
            Self::DirectionAlongSketch { .. } => {
                "Choose an edge, axis or line that leaves the sketch plane.".to_owned()
            }
            Self::EndAlongDirection | Self::EndAlong { .. } => {
                "Choose a face or plane that faces the sketch, or enter a distance.".to_owned()
            }
            Self::EndsCross { .. } => {
                "Choose ends that stay apart across the whole profile.".to_owned()
            }
            Self::AxisWithoutDirection => "Choose a line as the axis.".to_owned(),
            Self::BothSides { .. } => {
                "Keep one side of the axis, choose only the regions on one side, or choose \
                 another axis."
                    .to_owned()
            }
            Self::CrossesAxis { sketch, .. } => format!(
                "Keep one side of the axis, keep the profile on one side of it in {sketch}, or \
                 choose another axis."
            ),
            Self::OnAxis { .. } => "Choose another axis.".to_owned(),
            Self::RegionUnbuilt {
                sketch, several, ..
            } => {
                let which = if *several { "one of them" } else { "it" };
                format!("Change {which} in {sketch}, or leave that region out.")
            }
            Self::TaperUnsupported { sketch, .. } => {
                format!("Draw it with lines and arcs in {sketch}, or set the taper to 0°.")
            }
            Self::TaperTilted { .. } => {
                "Enter a distance, choose a parallel face or plane, or set the taper to 0°."
                    .to_owned()
            }
            Self::TaperCloses { shape, .. } => {
                format!("Make the taper angle smaller or the {shape} shorter.")
            }
            Self::Unbuilt { sketch, .. } => {
                format!("Undo the last change, or simplify the regions of {sketch}.")
            }
            Self::Vanishes { .. } => {
                "Change the distance or the sketch so that part of the body stays.".to_owned()
            }
            Self::Edgewise { .. } => {
                "Move or resize the shape so it overlaps more or stays clear.".to_owned()
            }
            Self::Uncombined { trouble, .. } => trouble.remedy(),
            Self::ExtrusionEmpty => "Enter distances that do not cancel each other out.".to_owned(),
            Self::TwoSidedStop { .. } => {
                "Choose One side, or use Up to face or a distance on that side.".to_owned()
            }
            Self::SurfaceFace { .. } => {
                "Select a face and use it for this end, or choose another end.".to_owned()
            }
            Self::EndNotMoved { offset, .. } => format!("Enter another {offset}."),
            Self::EndOffsetShort { label, .. } => format!(
                "Enter a smaller {label}, or choose a face or plane farther from the sketch."
            ),
            Self::ThroughAllAdds => {
                "Choose Remove from body or Intersect with body, or use Up to next or a distance."
                    .to_owned()
            }
            Self::NothingToGoThrough { side, .. } | Self::ReachNothing { side, .. } => {
                format!("{}, or enter a distance.", side.other_way())
            }
            Self::NextNeedsBody => {
                "Choose Add, Remove or Intersect with a body, or enter a distance.".to_owned()
            }
            Self::NextUnoffsettable { .. } => {
                "Clear the end offset, or use Up to face with a flat face or plane.".to_owned()
            }
            Self::AddInside { .. } => {
                "Choose Remove from body, or place the sketch outside the body.".to_owned()
            }
            Self::RemoveEntering { .. } => {
                "Place the sketch on or inside the body, or use Through all or Up to face."
                    .to_owned()
            }
            Self::ReachPartly { .. } => {
                "Use Up to face to choose where it stops, or keep the profile within the outline \
                 of the body."
                    .to_owned()
            }
            Self::ReachSeveral { .. } => {
                "Use Up to face to choose which one it stops at.".to_owned()
            }
            Self::ReachCurved { .. } => {
                "Use Up to face with a flat face or plane, or enter a distance.".to_owned()
            }
            Self::ReachUndecided { .. } => {
                "Use Up to face to choose where it stops, or enter a distance.".to_owned()
            }
            Self::SquareOnly { .. } => {
                "Run the extrusion square to the sketch, or use Up to face or a distance."
                    .to_owned()
            }
            Self::EndNotAhead { side, .. } => format!(
                "{}, or choose a face or plane beyond the sketch.",
                side.other_way()
            ),
            Self::EndCuts { .. } => {
                "Choose a face or plane that lies wholly beyond the profile.".to_owned()
            }
            Self::StopStraddles { .. } => {
                "Use Up to face or a distance, or move the sketch.".to_owned()
            }
            Self::StopEnclosed { .. } | Self::StopUnbuilt { .. } => {
                "Use Up to face or enter a distance.".to_owned()
            }
            Self::StopBeforeFace { .. } => {
                "Choose the face the profile meets first, or use Up to next.".to_owned()
            }
            Self::NotThroughAxis { .. } => {
                "Choose a face or plane through the axis, or enter an angle.".to_owned()
            }
            Self::WouldNotTurn { .. } => {
                "Choose another face or plane through the axis, or enter an angle.".to_owned()
            }
            Self::AnglesOverflow => "Enter angles that add up to at most 360°.".to_owned(),
            Self::DirectionAlong { .. } => {
                "Choose an edge, axis or line that leaves the sketch plane, or run the extrusion \
                 square to the sketch."
                    .to_owned()
            }
            Self::AxisOffPlane { .. } => {
                "Choose an axis that lies in the sketch plane, or a line of the sketch.".to_owned()
            }
            Self::NoSideLeft { .. } => {
                "Keep the other side, or choose regions on this side of the axis.".to_owned()
            }
            Self::NotALine { .. } => {
                "Choose a line or one of the sketch axes as the revolution axis.".to_owned()
            }
        }
    }

    fn gaps_text(gaps: &[Gap], unnamed: usize) -> Option<String> {
        let named: Vec<String> = gaps
            .iter()
            .map(|gap| {
                format!(
                    "an end of {} is {} from an end of {}",
                    gap.first,
                    Quantity::length(gap.distance),
                    gap.second
                )
            })
            .collect();
        named.first()?;
        let mut text = named.join(" and ");
        if unnamed > 0 {
            let noun = if unnamed == 1 { "gap" } else { "gaps" };
            text.push_str(&format!(", plus {unnamed} more {noun}"));
        }
        Some(text)
    }
}

impl From<SolidCause> for Cause {
    fn from(cause: SolidCause) -> Self {
        Self::Solid(cause)
    }
}
