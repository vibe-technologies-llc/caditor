use caditor_document::{
    AxisReference, DatumKind, Document, Evaluation, FeatureId, PlaneReference, PrincipalAxis,
    PrincipalPlane, SolidResult,
};
use caditor_geometry::Point2;
use caditor_kernel::{EdgeId, EdgeName, EdgeReference, FaceReference};
use serde::Deserialize;

use crate::{
    bodies, hole_tools,
    mcp::{
        references::{self, EntityName, Named, body_label},
        tools::ToolError,
        writing::PlaneName,
    },
    sketch_placement::{self, FaceChoice},
};

const EDGE_KEY_DIGITS: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PlaceError {
    #[error("“{0}” is not an edge key; use one list_edges gives")]
    UnreadableEdgeKey(String),
    #[error("{body} has no edge {edge} now; list_edges names its edges as they are")]
    NoEdge { body: String, edge: String },
    #[error("{what} cannot be used as a plane: {reason}")]
    NotAPlane { what: String, reason: String },
    #[error(
        "{what} has no axis: give a straight or round edge, a cylinder, cone or other round \
         face, a sketch line, a datum axis or X, Y or Z"
    )]
    NoAxis { what: String },
    #[error("{name} is not a datum {kind}; list_features names each datum")]
    NotADatum { name: String, kind: &'static str },
    #[error("{name} comes after the end of the tree, so a new feature cannot use it")]
    AfterTheBar { name: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum AxisName {
    X,
    Y,
    Z,
}

impl AxisName {
    fn principal(self) -> PrincipalAxis {
        match self {
            Self::X => PrincipalAxis::X,
            Self::Y => PrincipalAxis::Y,
            Self::Z => PrincipalAxis::Z,
        }
    }
}

pub fn principal_plane(name: PlaneName) -> PrincipalPlane {
    match name {
        PlaneName::Xy => PrincipalPlane::Xy,
        PlaneName::Xz => PrincipalPlane::Xz,
        PlaneName::Yz => PrincipalPlane::Yz,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OnFace {
    pub body: Named,
    pub face: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OnEdge {
    pub body: Named,
    pub edge: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SketchLine {
    pub sketch: Named,
    pub line: EntityName,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatumNamed {
    pub datum: Named,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum PlaneChoice {
    Principal(PlaneName),
    Face(OnFace),
    Datum(DatumNamed),
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum AxisChoice {
    Principal(AxisName),
    Edge(OnEdge),
    Face(OnFace),
    Sketch(SketchLine),
    Datum(DatumNamed),
}

pub fn edge_key_text(name: EdgeName) -> String {
    format!("{:0width$x}", name.digest(), width = EDGE_KEY_DIGITS)
}

pub fn edge_key(text: &str) -> Result<EdgeName, PlaceError> {
    let unreadable = || PlaceError::UnreadableEdgeKey(text.to_owned());
    let digits = text.trim();
    if digits.is_empty() || digits.len() > EDGE_KEY_DIGITS {
        return Err(unreadable());
    }
    let digest = u128::from_str_radix(digits, 16).map_err(|_| unreadable())?;
    Ok(EdgeName::from_digest(digest))
}

pub fn edge<'e>(
    document: &Document,
    evaluation: &'e Evaluation,
    body: FeatureId,
    text: &str,
) -> Result<(EdgeName, EdgeId, &'e SolidResult), ToolError> {
    let name = edge_key(text)?;
    let missing = || PlaceError::NoEdge {
        body: body_label(document, body),
        edge: text.to_owned(),
    };
    let shown = bodies::shown(evaluation, body).ok_or_else(missing)?;
    let found = bodies::find_edge(shown, name).ok_or_else(missing)?;
    Ok((name, found, shown))
}

fn datum(
    document: &Document,
    named: &Named,
    kind: DatumKind,
    word: &'static str,
) -> Result<FeatureId, ToolError> {
    let found = references::feature(document, named)?;
    let is_kind = found.kind.datum().is_some_and(|datum| datum.kind() == kind);
    if !is_kind {
        return Err(PlaceError::NotADatum {
            name: found.name.clone(),
            kind: word,
        }
        .into());
    }
    let before_the_bar = document
        .feature_index(found.id())
        .is_some_and(|index| index < document.bar_index());
    if !before_the_bar {
        return Err(PlaceError::AfterTheBar {
            name: found.name.clone(),
        }
        .into());
    }
    Ok(found.id())
}

pub fn plane(
    document: &Document,
    evaluation: &Evaluation,
    choice: &PlaneChoice,
) -> Result<PlaneReference, ToolError> {
    match choice {
        PlaneChoice::Principal(name) => Ok(PlaneReference::Principal(principal_plane(*name))),
        PlaneChoice::Datum(named) => {
            datum(document, &named.datum, DatumKind::Plane, "plane").map(PlaneReference::Datum)
        }
        PlaneChoice::Face(on) => on_face(document, evaluation, on).map(|placed| placed.reference),
    }
}

pub struct FacePlace {
    pub reference: PlaneReference,
    pub middle: Option<Point2>,
}

pub fn on_face(
    document: &Document,
    evaluation: &Evaluation,
    on: &OnFace,
) -> Result<FacePlace, ToolError> {
    let body = references::body(document, evaluation, &on.body)?;
    let (key, face, shown) = references::face(document, evaluation, body, &on.face)?;
    let (attachment, frame) = sketch_placement::attachment_in(
        document,
        evaluation,
        FaceChoice { body, face: key },
        document.bar_index(),
    )
    .map_err(|reason| PlaceError::NotAPlane {
        what: format!("face {} of {}", on.face, body_label(document, body)),
        reason: reason.to_owned(),
    })?;
    Ok(FacePlace {
        reference: PlaneReference::Face(attachment),
        middle: hole_tools::middle_of(&shown.solid, face, &frame),
    })
}

pub fn axis(
    document: &Document,
    evaluation: &Evaluation,
    choice: &AxisChoice,
) -> Result<AxisReference, ToolError> {
    let index = document.bar_index();
    match choice {
        AxisChoice::Principal(name) => Ok(AxisReference::Principal(name.principal())),
        AxisChoice::Datum(named) => {
            datum(document, &named.datum, DatumKind::Axis, "axis").map(AxisReference::Datum)
        }
        AxisChoice::Edge(on) => {
            let body = references::body(document, evaluation, &on.body)?;
            let (_, found, shown) = edge(document, evaluation, body, &on.edge)?;
            let no_axis = || PlaceError::NoAxis {
                what: format!("edge {} of {}", on.edge, body_label(document, body)),
            };
            let reference = EdgeReference::capture(&shown.solid, found).ok_or_else(no_axis)?;
            let state = sketch_placement::body_state_in(document, evaluation, body, index)
                .map_err(|_| no_axis())?;
            let there = reference.resolve(state).map_err(|_| no_axis())?;
            Ok(AxisReference::capture_edge(body, state, there).ok_or_else(no_axis)?)
        }
        AxisChoice::Face(on) => {
            let body = references::body(document, evaluation, &on.body)?;
            let (_, found, shown) = references::face(document, evaluation, body, &on.face)?;
            let no_axis = || PlaceError::NoAxis {
                what: format!("face {} of {}", on.face, body_label(document, body)),
            };
            let reference = FaceReference::capture(&shown.solid, found).ok_or_else(no_axis)?;
            let state = sketch_placement::body_state_in(document, evaluation, body, index)
                .map_err(|_| no_axis())?;
            let there = reference.resolve(state).map_err(|_| no_axis())?;
            Ok(AxisReference::capture_face(body, state, there).ok_or_else(no_axis)?)
        }
        AxisChoice::Sketch(line) => {
            let (feature, sketch) = references::sketch(document, &line.sketch)?;
            let entity = references::entity(&feature.name, sketch, &line.line)?;
            if sketch.line_endpoints(entity).is_none() {
                return Err(PlaceError::NoAxis {
                    what: format!("entity {} of {}", line.line, feature.name),
                }
                .into());
            }
            Ok(AxisReference::Sketch {
                sketch: feature.id(),
                entity,
            })
        }
    }
}
