use crate::{attachment::AttachmentError, failure::Cause};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Unplaced {
    PlaneThroughPoints,
    PlaneBetween,
    PlaneThroughAxisAndPoint,
    PlaneSquareToAxis,
    PlaneSquareToEdge,
    PlaneThroughLines,
    AxisThroughPoints,
    AxisSquareToPlane,
    Point,
    PointOffsets,
    CoordinateSystem,
    PlaneMoved,
    RotationAxis,
}

impl Unplaced {
    fn reason(self) -> &'static str {
        match self {
            Self::PlaneThroughPoints => "The plane could not be placed through its points.",
            Self::PlaneBetween => "The plane could not be placed between the two planes.",
            Self::PlaneThroughAxisAndPoint => {
                "The plane could not be placed through the axis and the point."
            }
            Self::PlaneSquareToAxis => "The plane could not be placed square to the axis.",
            Self::PlaneSquareToEdge => "The plane could not be placed square to the edge.",
            Self::PlaneThroughLines => "The plane could not be placed through the two lines.",
            Self::AxisThroughPoints => "The axis could not be placed through its points.",
            Self::AxisSquareToPlane => "The axis could not be placed square to the plane.",
            Self::Point | Self::PointOffsets => "The point could not be placed.",
            Self::CoordinateSystem => "The coordinate system could not be placed.",
            Self::PlaneMoved => "The plane could not be placed.",
            Self::RotationAxis => "The rotation axis has no direction.",
        }
    }

    fn remedy(self) -> &'static str {
        match self {
            Self::PlaneThroughPoints => "Choose three points farther apart.",
            Self::PlaneBetween => "Choose two other planes.",
            Self::PlaneThroughAxisAndPoint => "Choose another axis or point.",
            Self::PlaneSquareToAxis => "Choose another axis.",
            Self::PlaneSquareToEdge => "Change the distance or choose another edge.",
            Self::PlaneThroughLines => "Choose two other lines.",
            Self::AxisThroughPoints => "Choose two other points.",
            Self::AxisSquareToPlane => "Choose another plane.",
            Self::Point => "Choose other references.",
            Self::PointOffsets => "Change its offsets.",
            Self::CoordinateSystem => "Choose another point, axis or plane for it.",
            Self::PlaneMoved => "Change the offset or the angle.",
            Self::RotationAxis => "Choose another axis.",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Run {
    Missing,
    Tied,
    Gapped,
    PartlyTraced,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    PlaneOrAxis,
    Frame,
    Plane,
    Axis,
    Point,
}

impl Kind {
    fn words(self) -> &'static str {
        match self {
            Self::PlaneOrAxis => "a plane or an axis",
            Self::Frame => "a coordinate system",
            Self::Plane => "a plane",
            Self::Axis => "an axis",
            Self::Point => "a point",
        }
    }

    fn instead(self) -> &'static str {
        match self {
            Self::PlaneOrAxis => "Choose a plane or an axis instead.",
            Self::Frame => "Choose a coordinate system instead.",
            Self::Plane => "Choose a plane instead.",
            Self::Axis => "Choose an axis instead.",
            Self::Point => "Choose a point instead.",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Pair {
    pub first: String,
    pub second: String,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum DatumCause {
    Not {
        name: String,
        kind: Kind,
    },
    BasisFace {
        error: AttachmentError,
        body: String,
    },
    FramePartUnplaced {
        part: String,
        frame: String,
        plane: bool,
    },
    AxisNoDirection,
    EdgeGone {
        name: String,
    },
    EdgeBent {
        name: String,
    },
    AxisFaceGone {
        described: String,
        body: String,
    },
    FaceNotRound {
        described: String,
    },
    SketchLineFailed {
        name: String,
    },
    LineGone {
        name: String,
    },
    CornerGone {
        name: String,
    },
    CornerSeveral {
        name: String,
    },
    RoundEdgeGone {
        name: String,
    },
    EdgeNotRound {
        name: String,
    },
    PointFaceGone {
        described: String,
        body: String,
    },
    FaceNotSphere {
        described: String,
    },
    SketchPointFailed {
        name: String,
    },
    PointGone {
        name: String,
    },
    PointsCollinear,
    NotHalved {
        pair: Pair,
    },
    PointOnAxis {
        point: String,
        axis: String,
    },
    PlanesParallel {
        pair: Pair,
    },
    PointsCoincide {
        pair: Pair,
    },
    Unplaced(Unplaced),
    RunMissing {
        run: Run,
        name: String,
    },
    PastEnd {
        name: String,
        length: f64,
        distance: f64,
    },
    NoDirection {
        name: String,
    },
    FaceGone {
        described: String,
        body: String,
    },
    FaceMissing {
        described: String,
    },
    NotCurved {
        described: String,
    },
    OnAxis {
        described: String,
    },
    NoSurfaceDirection {
        described: String,
    },
    PlaneAgainst {
        described: String,
        picked: bool,
    },
    CentreNotFound {
        described: String,
    },
    FlatFace {
        described: String,
    },
    EvenSides {
        what: &'static str,
        described: String,
    },
    NoNormalNearest {
        described: String,
    },
    NearestNotFound {
        described: String,
    },
    AxisAgainst {
        described: String,
    },
    SameLine {
        pair: Pair,
    },
    LinesNotOnePlane {
        pair: Pair,
    },
    LinesParallel {
        pair: Pair,
    },
    LinesApart {
        pair: Pair,
    },
    LineAlongPlane {
        axis: String,
        plane: String,
        within: bool,
    },
    PlanesNoCorner {
        first: String,
        second: String,
        third: String,
    },
    XAxisAcross {
        axis: String,
        plane: String,
    },
    AxisNotAlong {
        axis: String,
        plane: String,
    },
}

impl DatumCause {
    pub(crate) fn reason(&self) -> String {
        match self {
            Self::Not { name, kind } => format!("{name} is not {}.", kind.words()),
            Self::BasisFace { error, body } => match error {
                AttachmentError::Missing => {
                    format!("The face it is based on is no longer part of {body}.")
                }
                AttachmentError::Ambiguous => format!(
                    "The face of {body} it is based on was split into parts that no longer lie \
                     in one plane."
                ),
                AttachmentError::NotFlat => {
                    format!("The face of {body} it is based on is no longer flat.")
                }
            },
            Self::FramePartUnplaced { part, frame, .. } => {
                format!("The {part} of {frame} could not be placed.")
            }
            Self::AxisNoDirection => "The axis has no direction.".to_owned(),
            Self::EdgeGone { name } => format!("The edge it uses is no longer part of {name}."),
            Self::EdgeBent { name } => {
                format!("The edge of {name} it uses is no longer straight.")
            }
            Self::AxisFaceGone { described, body } | Self::PointFaceGone { described, body } => {
                format!("{described} is no longer part of {body}.")
            }
            Self::FaceNotRound { described } => {
                format!("{described} is no longer round about an axis.")
            }
            Self::SketchLineFailed { name } => {
                format!("It uses a line of {name}, which has an error.")
            }
            Self::LineGone { name } => {
                format!("The line of {name} it uses no longer exists or is no longer a line.")
            }
            Self::CornerGone { name } => format!("The corner it uses is no longer part of {name}."),
            Self::CornerSeveral { name } => format!(
                "The corner of {name} it uses is now several corners, so it is unclear which one \
                 to follow."
            ),
            Self::RoundEdgeGone { name } => {
                format!("The round edge it uses is no longer part of {name}.")
            }
            Self::EdgeNotRound { name } => {
                format!("The edge of {name} it uses is no longer round.")
            }
            Self::FaceNotSphere { described } => {
                format!("{described} is no longer a sphere or a torus.")
            }
            Self::SketchPointFailed { name } => {
                format!("It uses a point of {name}, which has an error.")
            }
            Self::PointGone { name } => format!("The point of {name} it uses no longer exists."),
            Self::PointsCollinear => {
                "Its three points lie on one line, or two of them coincide, so no single plane \
                 passes through them."
                    .to_owned()
            }
            Self::NotHalved { pair } => {
                format!("{} and {} could not be halved.", pair.first, pair.second)
            }
            Self::PointOnAxis { point, axis } => {
                format!("{point} lies on {axis}, so no single plane is fixed by them.")
            }
            Self::PlanesParallel { pair } => format!(
                "{} and {} are parallel, so they do not meet in a line.",
                pair.first, pair.second
            ),
            Self::PointsCoincide { pair } => format!(
                "{} and {} are at the same place, so no single axis passes through them.",
                pair.first, pair.second
            ),
            Self::Unplaced(unplaced) => unplaced.reason().to_owned(),
            Self::RunMissing { run, name } => match run {
                Run::Missing => format!("The edge it follows is no longer part of {name}."),
                Run::Tied => format!("The edge of {name} it follows is now several edges."),
                Run::Gapped => format!(
                    "The edge of {name} it follows is now several pieces with gaps between them, \
                     so there is no one edge to follow."
                ),
                Run::PartlyTraced => format!(
                    "The edge of {name} it follows was split into pieces by a sketch edit, and \
                     only some of the pieces can still be traced back to it."
                ),
            },
            Self::PastEnd {
                name,
                length,
                distance,
            } => format!(
                "The edge of {name} it follows is only {length:.3} mm long, so a distance of \
                 {distance:.3} mm along it lies past its end."
            ),
            Self::NoDirection { name } => {
                format!("The edge of {name} it follows has no direction at that distance.")
            }
            Self::FaceGone { described, body } => {
                format!("{described} is no longer part of {body}.")
            }
            Self::FaceMissing { described } => {
                format!("{described} is no longer part of the body.")
            }
            Self::NotCurved { described } => {
                format!("{described} is no longer a cylindrical or conical face.")
            }
            Self::OnAxis { described } => format!(
                "The point that picks the side of {described} lies on its axis, so every side is \
                 equally near."
            ),
            Self::NoSurfaceDirection { described } => {
                format!("{described} has no surface direction at the chosen side.")
            }
            Self::PlaneAgainst { described, .. } => {
                format!("The plane could not be placed against {described}.")
            }
            Self::CentreNotFound { described } => {
                format!("The centre of {described} could not be found.")
            }
            Self::FlatFace { described } => {
                format!("{described} is flat, so it has no single point to touch.")
            }
            Self::EvenSides { what, described } => format!(
                "The chosen point lies {what} of {described}, so every side of it is equally near."
            ),
            Self::NoNormalNearest { described } => {
                format!("{described} has no surface direction nearest the chosen point.")
            }
            Self::NearestNotFound { described } => {
                format!("The point of {described} nearest the chosen point could not be found.")
            }
            Self::AxisAgainst { described } => {
                format!("The axis could not be placed square to {described}.")
            }
            Self::SameLine { pair } => format!(
                "{} and {} are the same line, so no single plane is fixed by them.",
                pair.first, pair.second
            ),
            Self::LinesNotOnePlane { pair } => format!(
                "{} and {} do not lie in one plane, since they neither cross nor run parallel.",
                pair.first, pair.second
            ),
            Self::LinesParallel { pair } => format!(
                "{} and {} run parallel, so they never cross.",
                pair.first, pair.second
            ),
            Self::LinesApart { pair } => format!(
                "{} and {} pass each other without meeting.",
                pair.first, pair.second
            ),
            Self::LineAlongPlane {
                axis,
                plane,
                within: true,
            } => format!("{axis} lies in {plane}, so it crosses it everywhere."),
            Self::LineAlongPlane {
                axis,
                plane,
                within: false,
            } => format!("{axis} runs parallel to {plane}, so it never crosses it."),
            Self::PlanesNoCorner {
                first,
                second,
                third,
            } => format!(
                "{first}, {second} and {third} do not meet in a single point, since two of them \
                 are parallel or all three share a line."
            ),
            Self::XAxisAcross { axis, plane } => format!(
                "{axis} stands square to {plane}, so it gives no direction in that plane for the \
                 X axis."
            ),
            Self::AxisNotAlong { axis, plane } => format!(
                "{axis} does not run along {plane}, so turning the plane about it cannot work."
            ),
        }
    }

    pub(crate) fn remedy(&self) -> String {
        match self {
            Self::Not { kind, .. } => kind.instead().to_owned(),
            Self::BasisFace { .. } => "Choose another plane or flat face for it.".to_owned(),
            Self::FramePartUnplaced { plane: true, .. } => "Choose another plane.".to_owned(),
            Self::FramePartUnplaced { plane: false, .. } => "Choose another axis.".to_owned(),
            Self::AxisNoDirection => "Choose another axis.".to_owned(),
            Self::EdgeGone { .. } => "Choose another edge or axis for it.".to_owned(),
            Self::EdgeBent { .. } => "Choose a straight edge or another axis for it.".to_owned(),
            Self::AxisFaceGone { .. } => "Choose another face or axis for it.".to_owned(),
            Self::FaceNotRound { .. } => {
                "Choose a cylindrical or conical face, or another axis.".to_owned()
            }
            Self::SketchLineFailed { name } | Self::SketchPointFailed { name } => {
                format!("Fix {name} first.")
            }
            Self::LineGone { .. } => "Choose another line or axis for it.".to_owned(),
            Self::CornerGone { .. } | Self::RoundEdgeGone { .. } | Self::PointGone { .. } => {
                "Choose another point for it.".to_owned()
            }
            Self::CornerSeveral { .. } => "Choose the point again.".to_owned(),
            Self::EdgeNotRound { .. } => {
                "Choose a circular edge or another point for it.".to_owned()
            }
            Self::PointFaceGone { .. } => "Choose another face or point for it.".to_owned(),
            Self::FaceNotSphere { .. } => {
                "Choose a spherical or toroidal face, or another point for it.".to_owned()
            }
            Self::PointsCollinear => "Choose three points that do not lie on one line.".to_owned(),
            Self::NotHalved { .. } => "Choose two other planes.".to_owned(),
            Self::PointOnAxis { .. } => "Choose a point off the axis.".to_owned(),
            Self::PlanesParallel { .. } => "Choose two planes or flat faces that cross.".to_owned(),
            Self::PointsCoincide { .. } => "Choose two points apart.".to_owned(),
            Self::Unplaced(unplaced) => unplaced.remedy().to_owned(),
            Self::RunMissing { run, .. } => match run {
                Run::Missing => "Choose another edge for it.".to_owned(),
                Run::Tied => "Choose the edge again.".to_owned(),
                Run::Gapped => {
                    "Choose the piece it should follow, or undo the edit that split the edge."
                        .to_owned()
                }
                Run::PartlyTraced => {
                    "Choose the edge again, or undo the sketch edit that split it.".to_owned()
                }
            },
            Self::PastEnd { .. } => "Shorten the distance, or choose a longer edge.".to_owned(),
            Self::NoDirection { .. } => "Change the distance.".to_owned(),
            Self::FaceGone { .. } | Self::FaceMissing { .. } => {
                "Choose another face for it.".to_owned()
            }
            Self::NotCurved { .. } => "Choose a cylindrical or conical face.".to_owned(),
            Self::OnAxis { .. } => "Choose a point off the axis of the face.".to_owned(),
            Self::NoSurfaceDirection { .. } => {
                "Choose a point farther from the tip of the cone.".to_owned()
            }
            Self::PlaneAgainst { picked: true, .. } => {
                "Choose another point to pick the side.".to_owned()
            }
            Self::PlaneAgainst { picked: false, .. } | Self::AxisAgainst { .. } => {
                "Choose another point.".to_owned()
            }
            Self::CentreNotFound { .. } => {
                "Choose another face, or place the point at a corner or along an edge.".to_owned()
            }
            Self::FlatFace { .. } => {
                "Choose a curved face, or base a plane on this face instead.".to_owned()
            }
            Self::EvenSides { .. } => "Choose a point off the middle of the face.".to_owned(),
            Self::NoNormalNearest { .. } => {
                "Choose a point nearer another part of the face.".to_owned()
            }
            Self::NearestNotFound { .. } => "Choose another point.".to_owned(),
            Self::SameLine { .. } => "Choose two lines that are not the same.".to_owned(),
            Self::LinesNotOnePlane { .. } => {
                "Choose two lines that cross or run parallel.".to_owned()
            }
            Self::LinesParallel { .. } => "Choose two lines that cross.".to_owned(),
            Self::LinesApart { .. } => {
                "Choose two lines that lie in one plane, or move one of them.".to_owned()
            }
            Self::LineAlongPlane { .. } => "Choose a line that cuts across the plane.".to_owned(),
            Self::PlanesNoCorner { .. } => {
                "Choose three planes or flat faces that cross each other in a corner.".to_owned()
            }
            Self::XAxisAcross { .. } => {
                "Choose an axis or straight edge that runs along the plane, or another plane."
                    .to_owned()
            }
            Self::AxisNotAlong { .. } => {
                "Choose an axis that lies in the plane or runs parallel to it, such as an edge \
                 of the face."
                    .to_owned()
            }
        }
    }
}

impl From<DatumCause> for Cause {
    fn from(cause: DatumCause) -> Self {
        Self::Datum(cause)
    }
}
