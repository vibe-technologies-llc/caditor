use caditor_kernel::{
    BendError, BuildError, FacetedError, GeometryError, LINEAR_RESOLUTION, TessellationError,
    ValidationError,
};

use crate::read::{graph::friendly, offset::OffsetRefusal, topology::short};

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum Fault {
    #[error("is missing from the file")]
    MissingFromFile,
    #[error("is not a {}", friendly(.0))]
    NotOfKind(String),
    #[error("combines several kinds")]
    CombinesKinds,
    #[error("has too few values")]
    TooFewValues,
    #[error("has a number that cannot be read")]
    UnreadableNumber,
    #[error("has a count that cannot be read")]
    UnreadableCount,
    #[error("has a value where a reference belongs")]
    ReferenceExpected,
    #[error("has a value where a list belongs")]
    ListExpected,
    #[error("has a value where true or false belongs")]
    LogicalExpected,
    #[error("could not be read")]
    Unreadable,
    #[error("is not a usable curve ({0})")]
    UnusableCurve(#[source] GeometryError),
    #[error("is not a usable surface ({0})")]
    UnusableSurface(#[source] GeometryError),
    #[error("is not a usable plane ({0})")]
    UnusablePlane(#[source] GeometryError),
    #[error("has a negative degree")]
    NegativeDegree,
    #[error("has degree {degree}, and caditor reads splines up to degree {max}")]
    DegreeTooHigh { degree: usize, max: usize },
    #[error("is part of a model too intricate to import in one go")]
    TooIntricate,
    #[error(
        "is a copy whose transformation cannot be read or scales it beyond what caditor can model"
    )]
    ReplicaUnplaceable,
    #[error("refers to itself")]
    SelfReference,
    #[error("is not a point in space")]
    NotAPoint,
    #[error("is a direction of no length")]
    ZeroDirection,
    #[error("is a vector of no length")]
    ZeroVector,
    #[error("has its reference direction along its axis")]
    ReferenceAlongAxis,
    #[error("is a {} edge, which caditor cannot import yet", friendly(.0))]
    UnsupportedEdge(String),
    #[error("is a {}, which caditor cannot import yet", friendly(.0))]
    Unsupported(String),
    #[error("is the inside of a torus whose tube just touches its axis, which holds no volume")]
    HornInside,
    #[error("is a piece of a curve with no end")]
    PieceOfEndlessCurve,
    #[error("is trimmed by nothing it can read")]
    TrimmedByNothing,
    #[error("is trimmed to nothing")]
    TrimmedToNothing,
    #[error("is offset along its own direction")]
    OffsetAlongOwnDirection,
    #[error("offsets a curve with no end")]
    OffsetOfEndlessCurve,
    #[error("is offset along its own direction somewhere")]
    OffsetAlongOwnDirectionSomewhere,
    #[error("has knots that do not match")]
    KnotsMismatch,
    #[error("has knots that do not fit its points")]
    KnotsMisfitPoints,
    #[error("has weights that do not match its points")]
    WeightsMismatchPoints,
    #[error("has weights that are not positive")]
    WeightsNotPositive,
    #[error("has weights that are not a list")]
    WeightsNotList,
    #[error("has a weight that cannot be read")]
    UnreadableWeight,
    #[error("has a row of points that is not a list")]
    PointRowNotList,
    #[error("has rows of points of different lengths")]
    RowsDiffer,
    #[error(transparent)]
    Offset(#[from] OffsetRefusal),
    #[error("lost its side while it was bent")]
    LostSideWhileBent,
    #[error("runs backwards along its bent face")]
    BackwardsAlongBentFace,
    #[error("touches other curved faces in a ring where each would have to give way to the next")]
    GiveWayRing,
    #[error("lies {off} mm from faces that cannot be bent to meet it")]
    VertexOffBentFaces { off: f64 },
    #[error("would have to move farther than the file's precision")]
    MovesBeyondPrecision,
    #[error("has an edge along a line of its surface that does not bound it")]
    EdgeAlongUnboundingLine,
    #[error("would have to bend farther than the file's precision to meet its neighbours")]
    BendsBeyondPrecision,
    #[error("has a side that cannot be followed")]
    SideUnfollowable,
    #[error("has an edge of no length along its side")]
    EdgeOfNoLengthAlongSide,
    #[error("has edges that overlap along its side")]
    EdgesOverlapAlongSide,
    #[error("could not be matched to the face it meets")]
    FaceUnmatched,
    #[error("meets a corner that lies off the face it is bent onto")]
    CornerOffBentFace,
    #[error("has edges along lines of its surface that leave no room between")]
    NoRoomBetweenLines,
    #[error("has two separate edges along lines that would both have to bound it on one side")]
    TwoLinesBoundOneSide,
    #[error("could not be cut down to its edges")]
    NotCutDownToEdges,
    #[error("was not bent, because the import was cancelled")]
    BendCancelled,
    #[error("could not be bent to meet its neighbours ({0})")]
    NotBent(#[source] BendError),
    #[error("has no closed shell, so it is not a solid")]
    NoClosedShell,
    #[error("has no surface")]
    NoSurface,
    #[error(
        "has faces {} mm apart, more than the file's precision of {} mm",
        short(*.loosest),
        short(*.allowance)
    )]
    FacesApart { loosest: f64, allowance: f64 },
    #[error("could not be meshed ({0})")]
    NotMeshed(#[source] TessellationError),
    #[error("did not close into one solid")]
    NotOneSolid,
    #[error("would need {faces} flat faces, too many to import")]
    TooManyFlatFaces { faces: usize },
    #[error("did not close into a solid ({0})")]
    NotClosedIntoSolid(#[source] FacetedError),
    #[error("{}", describe_build(.error, *.precision))]
    Build {
        error: BuildError,
        precision: Option<f64>,
    },
    #[error("was not checked, because the import was cancelled")]
    CheckCancelled,
    #[error("has face #{face} whose edges cross each other, so it does not enclose one volume")]
    FaceCrossesItself { face: u64 },
    #[error(
        "has faces #{first} and #{second} that cross each other, so it does not enclose one \
         volume"
    )]
    FacesCross { first: u64, second: u64 },
    #[error("has faces that cross each other, so it does not enclose one volume")]
    SomeFacesCross,
    #[error("declares no precision to move its faces within")]
    NoPrecisionToMove,
    #[error("has no face within the file's precision of meeting a neighbour exactly")]
    NoFaceToSnap,
    #[error("declares no precision to bend its faces within")]
    NoPrecisionToBend,
    #[error("is a {} where a closed shell belongs", friendly(.0))]
    ClosedShellExpected(String),
    #[error("is a {} where a face belongs", friendly(.0))]
    FaceExpected(String),
    #[error("is missing from its shell")]
    MissingFromShell,
    #[error("has no edges around it, which caditor cannot import yet")]
    NoEdgesAround,
    #[error("has no polygon to take its plane from")]
    NoPolygon,
    #[error("has a polygon that encloses no area")]
    PolygonWithoutArea,
    #[error("has fewer than three corners")]
    FewerThanThreeCorners,
    #[error("uses a vertex that is missing")]
    MissingVertex,
    #[error("runs backwards along its curve")]
    BackwardsAlongCurve,
}

impl Fault {
    pub(crate) fn build(error: BuildError, precision: Option<f64>) -> Self {
        log::debug!("a body could not be rebuilt: {error} ({error:?})");
        Self::Build { error, precision }
    }
}

fn describe_build(error: &BuildError, precision: Option<f64>) -> String {
    match error {
        BuildError::UnknownVertex(_)
        | BuildError::UnknownEdge(_)
        | BuildError::UnknownFace(_)
        | BuildError::UnknownShell(_) => {
            "refers to a part of the body that it does not contain".to_owned()
        }
        BuildError::NonFinitePoint => {
            "has a point whose position is not a finite number".to_owned()
        }
        BuildError::VertexOffCurve { distance, .. } => format!(
            "has a vertex {} mm away from the end of its edge",
            short(*distance)
        ),
        BuildError::ZeroLengthEdge => "has an edge of no length".to_owned(),
        BuildError::IntervalOutsideDomain => {
            "has an edge that runs past the end of its curve".to_owned()
        }
        BuildError::EmptyLoop => "has a face whose boundary has no edges".to_owned(),
        BuildError::TooManyEntities => "has more parts than caditor can hold".to_owned(),
        BuildError::Pcurve { .. } => {
            "has an edge that could not be followed across its face".to_owned()
        }
        BuildError::Geometry(_) => {
            "has a curve or surface that caditor cannot represent".to_owned()
        }
        BuildError::Invalid(validation) => describe_validation(validation, precision),
        BuildError::Cancelled(_) => "was not rebuilt, because the import was cancelled".to_owned(),
    }
}

fn describe_validation(error: &ValidationError, precision: Option<f64>) -> String {
    match error {
        ValidationError::EdgeOffSurface { distance, .. } => match precision {
            Some(precision) if *distance <= precision => format!(
                "has faces that meet only within {} mm, which the file's precision of {} mm \
                     allows, but caditor needs them to meet within {} mm, so the model would \
                     have to be exported again with a finer precision",
                short(*distance),
                short(precision),
                short(LINEAR_RESOLUTION)
            ),
            _ => format!(
                "has faces that meet only within {} mm, and caditor needs them to meet within \
                     {} mm",
                short(*distance),
                short(LINEAR_RESOLUTION)
            ),
        },
        ValidationError::Cancelled(_) => {
            "was not checked, because the import was cancelled".to_owned()
        }
        other => format!(
            "does not make a closed, valid solid: {}",
            invalid_reason(other)
        ),
    }
}

fn invalid_reason(error: &ValidationError) -> String {
    match error {
        ValidationError::NoShells => "it has no shell of faces".to_owned(),
        ValidationError::EmptyShell(_) => "one of its shells has no faces".to_owned(),
        ValidationError::FaceWithoutLoops(_) => "a face has no boundary".to_owned(),
        ValidationError::EmptyLoop(_) => "the boundary of a face has no edges".to_owned(),
        ValidationError::MissingEntity => "it refers to a part it does not contain".to_owned(),
        ValidationError::UnusedVertex(_) => "a vertex belongs to no edge".to_owned(),
        ValidationError::ZeroLengthEdge(_) => "an edge has no length".to_owned(),
        ValidationError::VertexOffEdge { distance, .. } => format!(
            "a vertex lies {} mm from the end of its edge",
            short(*distance)
        ),
        ValidationError::EdgeUseCount { uses, .. } => format!(
            "an edge is used {uses} times instead of by exactly two faces, so the faces do not close up"
        ),
        ValidationError::EdgeSenses(_) => {
            "two faces run the same way along a shared edge, so one of them faces the wrong way"
                .to_owned()
        }
        ValidationError::EdgeAcrossShells(_) => {
            "an edge joins faces of two separate shells".to_owned()
        }
        ValidationError::LoopBreak { .. } => {
            "the edges around a face do not join end to end".to_owned()
        }
        ValidationError::PcurveEnds(_)
        | ValidationError::PcurveGap(_)
        | ValidationError::PcurveOffEdge { .. } => {
            "an edge cannot be followed across its face".to_owned()
        }
        ValidationError::EdgeOffSurface { .. } => "an edge does not lie on its face".to_owned(),
        ValidationError::LoopOrientation { .. } => {
            "the boundary of a face winds the wrong way round it".to_owned()
        }
        ValidationError::InnerLoopOutside(_) => {
            "a hole in a face lies outside the face's outer boundary".to_owned()
        }
        ValidationError::ShellDisconnected { .. } => {
            "a face is not connected to the rest of its shell".to_owned()
        }
        ValidationError::EulerPoincare { .. } => {
            "its faces do not close up into one surface without gaps or extra edges".to_owned()
        }
        ValidationError::Tessellation(_) => {
            "its faces could not be turned into a mesh to check them".to_owned()
        }
        ValidationError::EmptyVolume(_) => "one of its shells encloses no volume".to_owned(),
        ValidationError::VoidOutside(_) => {
            "a shell facing inward lies inside no outer shell".to_owned()
        }
        ValidationError::LumpsOverlap { .. } => {
            "two of its shells overlap or lie inside one another".to_owned()
        }
        ValidationError::LumpsCoincide { .. } => {
            "two of its shells lie on top of each other".to_owned()
        }
        ValidationError::DanglingEdge { .. } => {
            "a face uses an edge twice at the same place".to_owned()
        }
        ValidationError::Cancelled(_) => {
            "it was not checked, because the import was cancelled".to_owned()
        }
    }
}
