use caditor_geometry::Point3;
use caditor_kernel::{BooleanError, FaceId, PatternError, Solid};

use crate::{
    datum::capitalized,
    describe::{describe_origin, lowercase_first},
    document::Document,
};

const MAX_NAMED_FACES: usize = 3;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Adjust {
    MoveIt,
    MoveThePlane,
    MoveThePlaneSlightly,
    MoveThePointOrChangeTheSizes,
    MoveOrResizeABody,
    MoveOrResize { shape: String },
    ChangeTheSpacingOrTheAngleSlightly,
    MoveItsCurves,
    Move { what: String },
    MoveTheCurveOf { sketch: String },
}

impl Adjust {
    pub(crate) fn words(&self) -> String {
        match self {
            Self::MoveIt => "Move it".to_owned(),
            Self::MoveThePlane => "Move the plane".to_owned(),
            Self::MoveThePlaneSlightly => "Move the plane slightly".to_owned(),
            Self::MoveThePointOrChangeTheSizes => "Move the point or change the sizes".to_owned(),
            Self::MoveOrResizeABody => "Move or resize a body".to_owned(),
            Self::MoveOrResize { shape } => format!("Move or resize the {shape}"),
            Self::ChangeTheSpacingOrTheAngleSlightly => {
                "Change the spacing or the angle slightly".to_owned()
            }
            Self::MoveItsCurves => "Move its curves".to_owned(),
            Self::Move { what } => format!("Move {what}"),
            Self::MoveTheCurveOf { sketch } => format!("Move the curve of {sketch}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TroubleKind {
    Intersection,
    Split,
    Ambiguous,
    Open,
    NonManifold,
    Invalid,
    Unlisted,
}

impl TroubleKind {
    fn of(error: &BooleanError) -> Self {
        match error {
            BooleanError::Intersection { .. } => Self::Intersection,
            BooleanError::Split(_) => Self::Split,
            BooleanError::Ambiguous(_) => Self::Ambiguous,
            BooleanError::Open(_) => Self::Open,
            BooleanError::NonManifold(_) => Self::NonManifold,
            BooleanError::Invalid(_) => Self::Invalid,
            _ => Self::Unlisted,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Trouble {
    kind: TroubleKind,
    ours: Vec<String>,
    theirs: Vec<String>,
    located: bool,
    adjust: Adjust,
    pub place: Option<Point3>,
}

impl Trouble {
    pub fn reason(&self, headline: String) -> String {
        match self.detail() {
            Some(detail) => format!("{headline} {detail}"),
            None => headline,
        }
    }

    pub fn remedy(&self) -> String {
        let adjust = self.adjust.words();
        match self.kind {
            TroubleKind::Invalid => format!("{adjust} slightly."),
            _ if self.located => format!(
                "{adjust} so the faces there line up exactly or stay clearly apart; faces a few \
                 micrometres apart count as neither."
            ),
            _ => {
                format!(
                    "{adjust} slightly; faces or edges that touch or nearly touch can cause this."
                )
            }
        }
    }

    fn detail(&self) -> Option<String> {
        let everything: Vec<String> = self.ours.iter().chain(&self.theirs).cloned().collect();
        let faces = listed(&everything);
        match (self.kind, faces) {
            (TroubleKind::Intersection, Some(_))
                if !self.ours.is_empty() && !self.theirs.is_empty() =>
            {
                Some(format!(
                    "Where {} meets {}, the faces could not be intersected.",
                    listed(&self.ours).unwrap_or_default(),
                    listed(&self.theirs).unwrap_or_default()
                ))
            }
            (TroubleKind::Intersection, Some(faces)) => {
                Some(format!("The faces at {faces} could not be intersected."))
            }
            (TroubleKind::Split, Some(faces)) => Some(format!(
                "{} could not be divided where the bodies meet.",
                capitalized(&faces)
            )),
            (TroubleKind::Ambiguous, Some(faces)) => Some(format!(
                "Where the bodies touch at {faces}, it cannot be told which side is inside."
            )),
            (TroubleKind::Ambiguous, None) => {
                Some("Where the bodies touch, it cannot be told which side is inside.".to_owned())
            }
            (TroubleKind::Open, Some(faces)) => Some(format!(
                "The faces of the result would not close up at {faces}."
            )),
            (TroubleKind::NonManifold, Some(faces)) => Some(format!(
                "The parts would meet only along an edge at {faces}."
            )),
            (TroubleKind::Invalid, _) => Some("The result would not be a valid solid.".to_owned()),
            _ => None,
        }
    }
}

pub(crate) fn place(error: &BooleanError) -> Option<Point3> {
    error.site().and_then(|site| site.point)
}

pub(crate) fn union_place(error: &PatternError) -> Option<Point3> {
    match error {
        PatternError::Union { error, .. } => place(error),
        PatternError::Placement { .. } | PatternError::Cancelled(_) => None,
    }
}

pub(crate) fn boolean_trouble(
    document: &Document,
    operands: [&Solid; 2],
    error: &BooleanError,
    adjust: Adjust,
) -> Trouble {
    let site = error.site();
    let [first, second] = operands;
    let names = |solid: &Solid, faces: &[FaceId]| -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        for face in faces {
            let name = lowercase_first(&describe_origin(
                document,
                solid.face(*face).and_then(|face| face.origin()),
            ));
            if !names.contains(&name) {
                names.push(name);
            }
        }
        names
    };
    let (ours, theirs) = site.map_or((Vec::new(), Vec::new()), |site| {
        (names(first, &site.first), names(second, &site.second))
    });
    Trouble {
        kind: TroubleKind::of(error),
        ours,
        theirs,
        located: site.is_some_and(|site| !site.is_empty()),
        adjust,
        place: site.and_then(|site| site.point),
    }
}

fn listed(names: &[String]) -> Option<String> {
    match names {
        [] => None,
        [only] => Some(only.clone()),
        [first, second] => Some(format!("{first} and {second}")),
        _ => {
            let shown = names.get(..MAX_NAMED_FACES).unwrap_or(names);
            let rest = names.len() - shown.len();
            match (shown, rest) {
                ([head @ .., last], 0) => Some(format!("{} and {last}", head.join(", "))),
                _ => Some(format!("{} and {rest} more", shown.join(", "))),
            }
        }
    }
}
