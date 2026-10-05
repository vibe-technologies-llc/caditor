use caditor_geometry::Point3;
use caditor_kernel::{BooleanError, FaceId, PatternError, Solid};

use crate::{
    datum::capitalized,
    describe::{describe_origin, lowercase_first},
    document::Document,
};

const MAX_NAMED_FACES: usize = 3;

pub(crate) struct Trouble {
    pub detail: Option<String>,
    pub remedy: String,
    pub place: Option<Point3>,
}

impl Trouble {
    pub fn reason(&self, headline: String) -> String {
        match &self.detail {
            Some(detail) => format!("{headline} {detail}"),
            None => headline,
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
    adjust: &str,
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
    let everything: Vec<String> = ours.iter().chain(&theirs).cloned().collect();
    let faces = listed(&everything);
    let detail = match (error, faces) {
        (BooleanError::Intersection { .. }, Some(_)) if !ours.is_empty() && !theirs.is_empty() => {
            Some(format!(
                "Where {} meets {}, the faces could not be intersected.",
                listed(&ours).unwrap_or_default(),
                listed(&theirs).unwrap_or_default()
            ))
        }
        (BooleanError::Intersection { .. }, Some(faces)) => {
            Some(format!("The faces at {faces} could not be intersected."))
        }
        (BooleanError::Split(_), Some(faces)) => Some(format!(
            "{} could not be divided where the bodies meet.",
            capitalized(&faces)
        )),
        (BooleanError::Ambiguous(_), Some(faces)) => Some(format!(
            "Where the bodies touch at {faces}, it cannot be told which side is inside."
        )),
        (BooleanError::Ambiguous(_), None) => {
            Some("Where the bodies touch, it cannot be told which side is inside.".to_owned())
        }
        (BooleanError::Open(_), Some(faces)) => Some(format!(
            "The faces of the result would not close up at {faces}."
        )),
        (BooleanError::NonManifold(_), Some(faces)) => Some(format!(
            "The parts would meet only along an edge at {faces}."
        )),
        (BooleanError::Invalid(_), _) => Some("The result would not be a valid solid.".to_owned()),
        _ => None,
    };
    let located = site.is_some_and(|site| !site.is_empty());
    let remedy = match error {
        BooleanError::Invalid(_) => format!("{adjust} slightly."),
        _ if located => format!(
            "{adjust} so the faces there line up exactly or stay clearly apart; faces a few \
             micrometres apart count as neither."
        ),
        _ => {
            format!("{adjust} slightly; faces or edges that touch or nearly touch can cause this.")
        }
    };
    Trouble {
        detail,
        remedy,
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
