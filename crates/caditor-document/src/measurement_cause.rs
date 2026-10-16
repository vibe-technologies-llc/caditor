use crate::{body_appearance::DensityError, failure::Cause, measurement::Of};

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Unreadable {
    MassWorked { body: String },
    Distance { between: String },
    NoAngle { between: String },
    NoDirectionAlong { axis: String },
    Offset { first: String, second: String },
    NoPosition { item: String },
    NoDirectionPosition { axis: String },
    Length { described: String },
    NoLongerRound { described: String },
    NoLongerCylinder { described: String },
    Area { described: String },
    NoSweep { described: String },
    Perimeter { described: String },
    NotBody { described: String, quantity: Of },
    NotArc { described: String },
    NotFace { described: String },
    NotEdgeOrCurve { described: String },
    NotRound { described: String },
    NotFaceOrBody { described: String },
    MassNotBody { item: String },
}

impl Unreadable {
    fn reason(&self) -> String {
        match self {
            Self::MassWorked { body } => {
                format!("The mass properties of {body} could not be worked out.")
            }
            Self::Distance { between } => {
                format!("The distance between {between} could not be worked out.")
            }
            Self::NoAngle { between } => format!("{between} have no angle between them."),
            Self::NoDirectionAlong { axis } => {
                format!("{axis} gives no direction, so nothing is measured along it.")
            }
            Self::Offset { first, second } => {
                format!("The offset between {first} and {second} could not be worked out.")
            }
            Self::NoPosition { item } => {
                format!("{item} is not a point or a body, so it has no position.")
            }
            Self::NoDirectionPosition { axis } => {
                format!("{axis} gives no direction, so no position is read along it.")
            }
            Self::Length { described } => {
                format!("The length of {described} could not be worked out.")
            }
            Self::NoLongerRound { described } => {
                format!("{described} is no longer round, so it has no radius.")
            }
            Self::NoLongerCylinder { described } => {
                format!("{described} is no longer a cylinder or a sphere, so it has no radius.")
            }
            Self::Area { described } => format!("The area of {described} could not be worked out."),
            Self::NoSweep { described } => {
                format!("{described} is no longer an arc, so it has no sweep.")
            }
            Self::Perimeter { described } => {
                format!("The perimeter of {described} could not be worked out.")
            }
            Self::NotBody {
                described,
                quantity,
            } => format!(
                "{described} is not a body, so it has no {}.",
                quantity.label().to_lowercase()
            ),
            Self::NotArc { described } => format!("{described} is not an arc, so it has no sweep."),
            Self::NotFace { described } => {
                format!("{described} is not a face, so it has no perimeter.")
            }
            Self::NotEdgeOrCurve { described } => {
                format!("{described} is not an edge or a curve, so it has no length.")
            }
            Self::NotRound { described } => {
                format!("{described} is not round, so it has no radius.")
            }
            Self::NotFaceOrBody { described } => {
                format!("{described} is not a face or a body, so it has no area.")
            }
            Self::MassNotBody { item } => format!("{item} is not a body, so it has no mass."),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum MeasureCause {
    Unreadable(Unreadable),
    EdgeGone { name: String },
    EdgeSeveral { name: String },
    EdgeSplit { name: String },
    FaceGone { described: String, name: String },
    FaceSeveral { described: String, name: String },
    SketchFailed { name: String },
    GeometryGone { name: String },
    NoDensity { name: String },
    BadDensity { name: String, error: DensityError },
}

impl MeasureCause {
    pub(crate) fn reason(&self) -> String {
        match self {
            Self::Unreadable(unreadable) => unreadable.reason(),
            Self::EdgeGone { name } => format!("The edge it measures is no longer part of {name}."),
            Self::EdgeSeveral { name } => format!(
                "The edge of {name} it measures is now several edges, so it is unclear which one \
                 to follow."
            ),
            Self::EdgeSplit { name } => format!(
                "The edge of {name} it measures was split into pieces by a sketch edit, and only \
                 some of the pieces still lie along it."
            ),
            Self::FaceGone { described, name } => {
                format!("{described} is no longer part of {name}.")
            }
            Self::FaceSeveral { described, name } => format!(
                "{described} is now several faces of {name}, so it is unclear which one to \
                 follow."
            ),
            Self::SketchFailed { name } => {
                format!("It measures geometry of {name}, which has an error.")
            }
            Self::GeometryGone { name } => {
                format!("The sketch geometry of {name} it measures no longer exists.")
            }
            Self::NoDensity { name } => {
                format!("{name} has no density, so its mass cannot be read.")
            }
            Self::BadDensity { name, error } => {
                format!("The mass of {name} cannot be read: {error}.")
            }
        }
    }

    pub(crate) fn remedy(&self) -> String {
        match self {
            Self::Unreadable(_) => {
                "Measure something that has this value, or delete this measurement.".to_owned()
            }
            Self::EdgeGone { .. } => {
                "Measure again with an edge that is there now, or delete this measurement."
                    .to_owned()
            }
            Self::EdgeSeveral { .. } => "Measure again with the edge you mean.".to_owned(),
            Self::EdgeSplit { .. } => {
                "Measure again with the edge or piece you mean, or undo the sketch edit that \
                 split it."
                    .to_owned()
            }
            Self::FaceGone { .. } => {
                "Measure again with a face that is there now, or delete this measurement."
                    .to_owned()
            }
            Self::FaceSeveral { .. } => "Measure again with the face you mean.".to_owned(),
            Self::SketchFailed { name } => format!("Fix {name} first."),
            Self::GeometryGone { .. } => {
                "Measure again with geometry that is there now, or delete this measurement."
                    .to_owned()
            }
            Self::NoDensity { name } => {
                format!("Give {name} a density in its colour and material.")
            }
            Self::BadDensity { name, .. } => {
                format!("Correct the density of {name} in its colour and material.")
            }
        }
    }
}

impl From<MeasureCause> for Cause {
    fn from(cause: MeasureCause) -> Self {
        Self::Measure(cause)
    }
}
