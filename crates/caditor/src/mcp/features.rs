use std::fmt;

use caditor_document::{
    AxisTurn, Blend, BlendKind, BodyOperation, ChamferForm, CircularPattern, Combine,
    CombineOperation, Datum, DatumAxis, DatumPlane, Document, EdgeGroup, Edit, FeatureId,
    FeatureKind, LinearDirection, LinearSpacing, Mirror, Move, OffsetFace, Pattern, PatternKind,
    PlaneReference, PlaneRotation, Primitive, PrimitiveAnchor, PrimitiveKind, Revolve, RevolveAxis,
    RevolveExtent, Shell, ShellSide, SolidFeature, TurnCentre, repeatable_on,
};
use caditor_expression::{Dimension, Expression};
use caditor_geometry::Point2;
use caditor_kernel::{EdgeNaming, EdgeReference, FaceReference};
use caditor_sketch::EntityId;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::{
    combine_tools, editing,
    field::{self, Expected},
    mcp::{
        places::{self, AxisChoice, PlaneChoice},
        references::{self, EntityName, Named, body_label},
        session::{Session, Work},
        tools::{Answer, ToolError},
        writing::{ModellingError, OperationName, apply, operation_words, with_feature},
    },
    mirror_tools, move_tools, offset_face_tools, pattern_tools, primitive_tools, shell_tools,
    solid_tools::{self, Reach},
    units::{LengthUnit, Units},
    visibility,
};

const FULL_TURN_DEGREES: f64 = 360.0;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum Amount {
    Number(f64),
    Text(String),
}

impl fmt::Display for Amount {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Number(number) => write!(formatter, "{number}"),
            Self::Text(text) => formatter.write_str(text),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum RevolveAxisChoice {
    Model(AxisChoice),
    Line(EntityName),
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Revolution {
    pub sketch: Named,
    pub axis: Option<RevolveAxisChoice>,
    pub angle: Option<Amount>,
    #[serde(default)]
    pub symmetric: bool,
    #[serde(default)]
    pub reversed: bool,
    pub operation: Option<OperationName>,
    pub body: Option<Named>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Filleting {
    pub body: Named,
    #[serde(default)]
    pub edges: Vec<String>,
    #[serde(default)]
    pub faces: Vec<String>,
    #[serde(default)]
    pub all_edges: bool,
    pub radius: Amount,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Chamfering {
    pub body: Named,
    #[serde(default)]
    pub edges: Vec<String>,
    #[serde(default)]
    pub faces: Vec<String>,
    #[serde(default)]
    pub all_edges: bool,
    pub distance: Amount,
    pub second_distance: Option<Amount>,
    pub angle: Option<Amount>,
    #[serde(default)]
    pub flipped: bool,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SideName {
    Inside,
    Outside,
    BothSides,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Shelling {
    pub body: Named,
    #[serde(default)]
    pub open_faces: Vec<String>,
    pub thickness: Amount,
    pub side: Option<SideName>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FaceOffset {
    pub body: Named,
    pub faces: Vec<String>,
    pub distance: Amount,
    #[serde(default)]
    pub tangent: bool,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrimitiveName {
    Box,
    Cylinder,
    Sphere,
    Torus,
    Cone,
    Wedge,
    Prism,
}

impl PrimitiveName {
    fn kind(self) -> PrimitiveKind {
        match self {
            Self::Box => PrimitiveKind::Box,
            Self::Cylinder => PrimitiveKind::Cylinder,
            Self::Sphere => PrimitiveKind::Sphere,
            Self::Torus => PrimitiveKind::Torus,
            Self::Cone => PrimitiveKind::Cone,
            Self::Wedge => PrimitiveKind::Wedge,
            Self::Prism => PrimitiveKind::Prism,
        }
    }

    fn sizes(self) -> &'static [&'static str] {
        match self {
            Self::Box => &["length", "width", "height"],
            Self::Cylinder => &["diameter", "height"],
            Self::Sphere => &["diameter"],
            Self::Torus => &["diameter", "tube"],
            Self::Cone => &["bottom", "top", "height"],
            Self::Wedge => &["length", "width", "height", "top_length"],
            Self::Prism => &["sides", "diameter", "height"],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnchorName {
    Corner,
    BaseCentre,
    Centre,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewPrimitive {
    pub kind: PrimitiveName,
    pub length: Option<Amount>,
    pub width: Option<Amount>,
    pub height: Option<Amount>,
    pub diameter: Option<Amount>,
    pub tube: Option<Amount>,
    pub bottom: Option<Amount>,
    pub top: Option<Amount>,
    pub top_length: Option<Amount>,
    pub sides: Option<Amount>,
    pub plane: Option<PlaneChoice>,
    pub at: Option<[Amount; 2]>,
    pub anchor: Option<AnchorName>,
    #[serde(default)]
    pub reversed: bool,
    pub operation: Option<OperationName>,
    pub body: Option<Named>,
    pub name: Option<String>,
}

impl NewPrimitive {
    fn given_sizes(&self) -> Vec<(&'static str, &Amount)> {
        [
            ("length", &self.length),
            ("width", &self.width),
            ("height", &self.height),
            ("diameter", &self.diameter),
            ("tube", &self.tube),
            ("bottom", &self.bottom),
            ("top", &self.top),
            ("top_length", &self.top_length),
            ("sides", &self.sides),
        ]
        .into_iter()
        .filter_map(|(word, amount)| amount.as_ref().map(|amount| (word, amount)))
        .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mirroring {
    pub body: Option<Named>,
    #[serde(default)]
    pub features: Vec<Named>,
    pub plane: PlaneChoice,
    pub keep_original: Option<bool>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Direction {
    pub axis: AxisChoice,
    pub count: Amount,
    pub spacing: Amount,
    #[serde(default)]
    pub total: bool,
    #[serde(default)]
    pub reversed: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinearPatterning {
    pub body: Option<Named>,
    #[serde(default)]
    pub features: Vec<Named>,
    pub axis: AxisChoice,
    pub count: Amount,
    pub spacing: Amount,
    #[serde(default)]
    pub total: bool,
    #[serde(default)]
    pub reversed: bool,
    pub second: Option<Direction>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CircularPatterning {
    pub body: Option<Named>,
    #[serde(default)]
    pub features: Vec<Named>,
    pub axis: AxisChoice,
    pub count: Amount,
    pub angle: Option<Amount>,
    #[serde(default)]
    pub reversed: bool,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CombineName {
    Join,
    Cut,
    Intersect,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Combining {
    pub body: Named,
    pub tools: Vec<Named>,
    pub operation: CombineName,
    #[serde(default)]
    pub keep_tools: bool,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Moving {
    pub body: Named,
    pub offset: Option<[Amount; 3]>,
    pub turn: Option<[Amount; 3]>,
    pub turn_axis: Option<AxisChoice>,
    pub turn_angle: Option<Amount>,
    #[serde(default)]
    pub copy: bool,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatumPlaning {
    pub base: PlaneChoice,
    pub offset: Option<Amount>,
    pub turn_axis: Option<AxisChoice>,
    pub turn_angle: Option<Amount>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatumAxising {
    pub along: Option<AxisChoice>,
    pub planes: Option<[PlaneChoice; 2]>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FeatureError {
    #[error(
        "say which edges to {verb}: edge keys from list_edges, face keys from list_faces (every \
         edge around those faces) or all_edges"
    )]
    NoEdges { verb: &'static str },
    #[error("give no edges or faces with all_edges, which takes every edge of the body")]
    AllEdgesAndMore,
    #[error("a chamfer takes a second_distance or an angle, not both")]
    TwoChamferForms,
    #[error("give the faces to move, as keys from list_faces")]
    NoFacesToOffset,
    #[error("{kind} takes {sizes}, not {size}")]
    SizeNotTaken {
        kind: &'static str,
        sizes: String,
        size: &'static str,
    },
    #[error("give the body to {verb}, or the features to repeat")]
    NoSubject { verb: &'static str },
    #[error("give either the body or the features to repeat, not both")]
    BodyAndFeatures,
    #[error(
        "{name} cannot be repeated on its own: only features that cut into or add to one body \
         (holes, cuts, bosses, primitives) can, and all of them on the same body"
    )]
    NotRepeatable { name: String },
    #[error("give at least one body to combine with {body}")]
    NoTools { body: String },
    #[error("{body} cannot be combined with itself")]
    CombinedWithItself { body: String },
    #[error("a turn about an axis takes both turn_axis and turn_angle")]
    HalfAnAxisTurn,
    #[error("turn about the body's own axes or about one axis, not both")]
    TwoTurns,
    #[error("a datum axis goes along an axis or where two planes meet: give along or planes")]
    NoDatumAxis,
    #[error("give along or planes for a datum axis, not both")]
    TwoDatumAxes,
}

fn amount(
    session: &Session,
    field_name: &'static str,
    given: &Amount,
    dimension: Dimension,
    non_negative: bool,
) -> Result<Expression, ToolError> {
    let text = given.to_string();
    field::parse_expression(
        session.document(),
        &session.parameters(),
        &text,
        Expected {
            dimension: Some(dimension),
            non_negative,
        },
        Units::default(),
    )
    .map_err(|reason| ToolError::Value {
        field: field_name,
        text,
        reason,
    })
}

fn length(
    session: &Session,
    field_name: &'static str,
    given: &Amount,
) -> Result<Expression, ToolError> {
    amount(session, field_name, given, Dimension::LENGTH, true)
}

fn signed_length(
    session: &Session,
    field_name: &'static str,
    given: &Amount,
) -> Result<Expression, ToolError> {
    amount(session, field_name, given, Dimension::LENGTH, false)
}

fn turn(
    session: &Session,
    field_name: &'static str,
    given: &Amount,
) -> Result<Expression, ToolError> {
    amount(session, field_name, given, Dimension::ANGLE, false)
}

fn count(
    session: &Session,
    field_name: &'static str,
    given: &Amount,
) -> Result<Expression, ToolError> {
    amount(session, field_name, given, Dimension::NONE, true)
}

fn feature_name(document: &Document, asked: Option<String>, title: &str) -> String {
    asked.unwrap_or_else(|| editing::next_feature_name(document, title))
}

fn added(
    session: &mut Session,
    work: &Work<'_>,
    (name, kind): (String, FeatureKind),
    hide: Option<FeatureId>,
    details: Map<String, Value>,
) -> Result<Answer, ToolError> {
    let document = session.document();
    let mut transaction = document.transaction(format!("Create {name}"));
    let feature = transaction.add_feature(name, kind);
    if let Some(sketch) = hide.filter(|sketch| visibility::is_shown(document, *sketch)) {
        transaction.edit(Edit::SetFeatureHidden {
            id: sketch,
            hidden: true,
        });
    }
    let transaction = transaction.finish();
    let answer = apply(session, work, transaction, details)?;
    Ok(with_feature(session, answer, feature))
}

fn chosen_operation(
    document: &Document,
    name: OperationName,
    target: Option<FeatureId>,
) -> Result<BodyOperation, ModellingError> {
    let target = target.or_else(|| document.bodies_standing().last().copied());
    match (name, target) {
        (OperationName::NewBody, _) => Ok(BodyOperation::NewBody),
        (OperationName::Add, Some(body)) => Ok(BodyOperation::Add(body)),
        (OperationName::Remove, Some(body)) => Ok(BodyOperation::Remove(body)),
        (OperationName::Intersect, Some(body)) => Ok(BodyOperation::Intersect(body)),
        (OperationName::Add, None) => Err(ModellingError::NoTarget { operation: "add" }),
        (OperationName::Remove, None) => Err(ModellingError::NoTarget {
            operation: "remove",
        }),
        (OperationName::Intersect, None) => Err(ModellingError::NoTarget {
            operation: "intersect",
        }),
    }
}

fn operation_details(document: &Document, operation: BodyOperation) -> Map<String, Value> {
    let mut details = Map::new();
    details.insert(
        "operation".to_owned(),
        json!(operation_words(document, operation)),
    );
    details
}

pub fn revolve(
    session: &mut Session,
    work: &Work<'_>,
    asked: Revolution,
) -> Result<Answer, ToolError> {
    let angle = asked
        .angle
        .as_ref()
        .map(|angle| turn(session, "angle", angle))
        .transpose()?;
    session.evaluate(work)?;
    let values = session.parameters();
    let document = session.document();
    let evaluation = session.evaluation();
    let (feature, sketch) = references::sketch(document, &asked.sketch)?;
    let axis = match &asked.axis {
        None => RevolveAxis::Sketch(EntityId::VERTICAL_AXIS),
        Some(RevolveAxisChoice::Line(line)) => {
            RevolveAxis::Sketch(references::entity(&feature.name, sketch, line)?)
        }
        Some(RevolveAxisChoice::Model(choice)) => {
            RevolveAxis::Model(places::axis(document, evaluation, choice)?)
        }
    };
    let extent = match (angle, asked.symmetric) {
        (None, _) => RevolveExtent::Full,
        (Some(angle), true) => RevolveExtent::Symmetric { angle },
        (Some(angle), false) => RevolveExtent::OneSide {
            angle,
            reversed: asked.reversed,
        },
    };
    let solid = SolidFeature::Revolve(Revolve {
        sketch: feature.id(),
        regions: caditor_document::RegionChoice::All,
        axis,
        extent,
        operation: BodyOperation::NewBody,
        start: None,
        other_bodies: Vec::new(),
        side: None,
        wall: None,
    });
    let chosen = asked
        .body
        .as_ref()
        .map(|body| references::body(document, evaluation, body))
        .transpose()?;
    let operation = match asked.operation {
        Some(name) => chosen_operation(document, name, chosen)?,
        None => match chosen {
            Some(body) => BodyOperation::Add(body),
            None => solid_tools::default_operation(
                document,
                Reach {
                    evaluation,
                    parameters: &values,
                },
                &solid,
            ),
        },
    };
    let solid = solid_tools::with_operation(&solid, operation);
    let name = feature_name(document, asked.name, solid_tools::Sweep::Revolve.label());
    let details = operation_details(document, operation);
    let sketch = feature.id();
    added(
        session,
        work,
        (name, FeatureKind::Solid(solid)),
        Some(sketch),
        details,
    )
}

struct EdgeChoice<'a> {
    body: &'a Named,
    edges: &'a [String],
    faces: &'a [String],
    all_edges: bool,
}

fn blend(
    session: &mut Session,
    work: &Work<'_>,
    kind: BlendKind,
    choice: EdgeChoice<'_>,
    (size, form, flipped): (Expression, ChamferForm, bool),
    name: Option<String>,
) -> Result<Answer, ToolError> {
    let verb = kind.noun();
    if choice.all_edges && !(choice.edges.is_empty() && choice.faces.is_empty()) {
        return Err(FeatureError::AllEdgesAndMore.into());
    }
    if !choice.all_edges && choice.edges.is_empty() && choice.faces.is_empty() {
        return Err(FeatureError::NoEdges { verb }.into());
    }
    session.evaluate(work)?;
    let document = session.document();
    let evaluation = session.evaluation();
    let body = references::body(document, evaluation, choice.body)?;
    let mut edges = Vec::with_capacity(choice.edges.len());
    for key in choice.edges {
        let (_, edge, shown) = places::edge(document, evaluation, body, key)?;
        let naming = EdgeNaming::new(&shown.solid);
        let reference =
            EdgeReference::capture_in(&naming, edge).ok_or_else(|| places::PlaceError::NoEdge {
                body: body_label(document, body),
                edge: key.clone(),
            })?;
        edges.push(reference);
    }
    let mut groups = Vec::with_capacity(choice.faces.len());
    for key in choice.faces {
        let (_, face, shown) = references::face(document, evaluation, body, key)?;
        let reference = FaceReference::capture(&shown.solid, face).ok_or_else(|| {
            references::ReferenceError::NoFace {
                body: body_label(document, body),
                face: key.clone(),
            }
        })?;
        groups.push(EdgeGroup::Face(reference));
    }
    if choice.all_edges {
        groups.push(EdgeGroup::Body);
    }
    let mut details = Map::new();
    details.insert("body".to_owned(), json!(body_label(document, body)));
    let name = feature_name(document, name, kind.title());
    let blend = Blend {
        kind,
        body,
        edges,
        groups,
        size,
        form,
        flipped,
    };
    added(
        session,
        work,
        (name, FeatureKind::Blend(blend)),
        None,
        details,
    )
}

pub fn fillet(
    session: &mut Session,
    work: &Work<'_>,
    asked: Filleting,
) -> Result<Answer, ToolError> {
    let radius = length(session, "radius", &asked.radius)?;
    blend(
        session,
        work,
        BlendKind::Fillet,
        EdgeChoice {
            body: &asked.body,
            edges: &asked.edges,
            faces: &asked.faces,
            all_edges: asked.all_edges,
        },
        (radius, ChamferForm::Equal, false),
        asked.name,
    )
}

pub fn chamfer(
    session: &mut Session,
    work: &Work<'_>,
    asked: Chamfering,
) -> Result<Answer, ToolError> {
    let distance = length(session, "distance", &asked.distance)?;
    let form = match (&asked.second_distance, &asked.angle) {
        (Some(_), Some(_)) => return Err(FeatureError::TwoChamferForms.into()),
        (Some(second), None) => ChamferForm::TwoDistances {
            second: length(session, "second_distance", second)?,
        },
        (None, Some(angle)) => ChamferForm::DistanceAngle {
            angle: amount(session, "angle", angle, Dimension::ANGLE, true)?,
        },
        (None, None) => ChamferForm::Equal,
    };
    blend(
        session,
        work,
        BlendKind::Chamfer,
        EdgeChoice {
            body: &asked.body,
            edges: &asked.edges,
            faces: &asked.faces,
            all_edges: asked.all_edges,
        },
        (distance, form, asked.flipped),
        asked.name,
    )
}

fn face_references(
    session: &Session,
    body: FeatureId,
    keys: &[String],
) -> Result<Vec<FaceReference>, ToolError> {
    let document = session.document();
    let evaluation = session.evaluation();
    keys.iter()
        .map(|key| {
            let (_, face, shown) = references::face(document, evaluation, body, key)?;
            FaceReference::capture(&shown.solid, face).ok_or_else(|| {
                references::ReferenceError::NoFace {
                    body: body_label(document, body),
                    face: key.clone(),
                }
                .into()
            })
        })
        .collect()
}

pub fn shell(session: &mut Session, work: &Work<'_>, asked: Shelling) -> Result<Answer, ToolError> {
    let thickness = length(session, "thickness", &asked.thickness)?;
    session.evaluate(work)?;
    let body = references::body(session.document(), session.evaluation(), &asked.body)?;
    let open = face_references(session, body, &asked.open_faces)?;
    let side = match asked.side {
        None | Some(SideName::Inside) => ShellSide::Inside,
        Some(SideName::Outside) => ShellSide::Outside,
        Some(SideName::BothSides) => ShellSide::BothSides,
    };
    let document = session.document();
    let mut details = Map::new();
    details.insert("body".to_owned(), json!(body_label(document, body)));
    let name = feature_name(document, asked.name, shell_tools::TITLE);
    added(
        session,
        work,
        (
            name,
            FeatureKind::Shell(Shell {
                body,
                open,
                thickness,
                side,
                faces: Vec::new(),
            }),
        ),
        None,
        details,
    )
}

pub fn offset_face(
    session: &mut Session,
    work: &Work<'_>,
    asked: FaceOffset,
) -> Result<Answer, ToolError> {
    if asked.faces.is_empty() {
        return Err(FeatureError::NoFacesToOffset.into());
    }
    let distance = signed_length(session, "distance", &asked.distance)?;
    session.evaluate(work)?;
    let body = references::body(session.document(), session.evaluation(), &asked.body)?;
    let faces = face_references(session, body, &asked.faces)?;
    let document = session.document();
    let mut details = Map::new();
    details.insert("body".to_owned(), json!(body_label(document, body)));
    let name = feature_name(document, asked.name, offset_face_tools::TITLE);
    added(
        session,
        work,
        (
            name,
            FeatureKind::OffsetFace(OffsetFace {
                body,
                faces,
                distance,
                tangent: asked.tangent,
            }),
        ),
        None,
        details,
    )
}

pub fn primitive(
    session: &mut Session,
    work: &Work<'_>,
    asked: NewPrimitive,
) -> Result<Answer, ToolError> {
    let kind = asked.kind.kind();
    let mut shape = primitive_tools::default_shape(kind, LengthUnit::Millimetre);
    let names = asked.kind.sizes();
    for (size, given) in asked.given_sizes() {
        let Some(position) = names.iter().position(|name| *name == size) else {
            return Err(FeatureError::SizeNotTaken {
                kind: kind.name(),
                sizes: names.join(", "),
                size,
            }
            .into());
        };
        let parsed = if size == "sides" {
            count(session, size, given)?
        } else {
            length(session, size, given)?
        };
        if let Some(slot) = shape.sizes_mut().into_iter().nth(position) {
            *slot = parsed;
        }
    }
    let at = match &asked.at {
        Some([x, y]) => Some([
            signed_length(session, "at", x)?,
            signed_length(session, "at", y)?,
        ]),
        None => None,
    };
    session.evaluate(work)?;
    let document = session.document();
    let evaluation = session.evaluation();
    let (plane, face_body, middle) = match &asked.plane {
        None => (
            PlaneReference::Principal(caditor_document::PrincipalPlane::Xy),
            None,
            None,
        ),
        Some(PlaneChoice::Face(on)) => {
            let placed = places::on_face(document, evaluation, on)?;
            (placed.reference, Some(placed.body), placed.middle)
        }
        Some(choice) => (places::plane(document, evaluation, choice)?, None, None),
    };
    let at = at.unwrap_or_else(|| {
        let spot = middle.unwrap_or(Point2::ZERO);
        [
            LengthUnit::Millimetre.measured(spot.x),
            LengthUnit::Millimetre.measured(spot.y),
        ]
    });
    let chosen = asked
        .body
        .as_ref()
        .map(|body| references::body(document, evaluation, body))
        .transpose()?;
    let operation = match asked.operation {
        Some(name) => chosen_operation(document, name, chosen.or(face_body))?,
        None => chosen
            .or(face_body)
            .map_or(BodyOperation::NewBody, BodyOperation::Add),
    };
    let cutting_into_its_face = face_body.is_some()
        && matches!(
            operation,
            BodyOperation::Remove(_) | BodyOperation::Intersect(_)
        );
    let primitive = Primitive {
        shape,
        plane,
        at,
        anchor: match asked.anchor {
            Some(AnchorName::Corner) => PrimitiveAnchor::Corner,
            Some(AnchorName::BaseCentre) => PrimitiveAnchor::BaseCentre,
            Some(AnchorName::Centre) => PrimitiveAnchor::Centre,
            None => primitive_tools::default_anchor(kind),
        },
        reversed: asked.reversed != cutting_into_its_face,
        operation,
    };
    let details = operation_details(document, operation);
    let name = feature_name(document, asked.name, kind.name());
    added(
        session,
        work,
        (name, FeatureKind::Primitive(primitive)),
        None,
        details,
    )
}

fn subject(
    session: &Session,
    body: Option<&Named>,
    features: &[Named],
    verb: &'static str,
) -> Result<(FeatureId, Vec<FeatureId>), ToolError> {
    let document = session.document();
    let evaluation = session.evaluation();
    match (body, features) {
        (Some(_), [_, ..]) => Err(FeatureError::BodyAndFeatures.into()),
        (None, []) => Err(FeatureError::NoSubject { verb }.into()),
        (Some(body), []) => Ok((references::body(document, evaluation, body)?, Vec::new())),
        (None, features) => {
            let mut found = Vec::with_capacity(features.len());
            for named in features {
                let feature = references::feature(document, named)?;
                if repeatable_on(&feature.kind).is_none() {
                    return Err(FeatureError::NotRepeatable {
                        name: feature.name.clone(),
                    }
                    .into());
                }
                found.push(feature.id());
            }
            pattern_tools::repeatable(document, &found).ok_or_else(|| {
                FeatureError::NotRepeatable {
                    name: features
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", "),
                }
                .into()
            })
        }
    }
}

fn subject_details(
    document: &Document,
    body: FeatureId,
    repeated: &[FeatureId],
) -> Map<String, Value> {
    let mut details = Map::new();
    details.insert(
        "repeats".to_owned(),
        json!(pattern_tools::subject(document, body, repeated)),
    );
    details
}

pub fn mirror(
    session: &mut Session,
    work: &Work<'_>,
    asked: Mirroring,
) -> Result<Answer, ToolError> {
    session.evaluate(work)?;
    let (body, mirrored) = subject(session, asked.body.as_ref(), &asked.features, "mirror")?;
    let document = session.document();
    let plane = places::plane(document, session.evaluation(), &asked.plane)?;
    let mut mirror = Mirror::new(body, plane).mirroring(mirrored.clone());
    if let Some(keep) = asked.keep_original {
        mirror.keep_original = keep;
    }
    let title = if mirrored.is_empty() {
        mirror_tools::TITLE
    } else {
        mirror_tools::FEATURES_TITLE
    };
    let details = subject_details(document, body, &mirrored);
    let name = feature_name(document, asked.name, title);
    added(
        session,
        work,
        (name, FeatureKind::Mirror(mirror)),
        None,
        details,
    )
}

fn direction(
    session: &Session,
    axis: &AxisChoice,
    (count_given, spacing, total, reversed): (&Amount, &Amount, bool, bool),
) -> Result<LinearDirection, ToolError> {
    Ok(LinearDirection {
        axis: places::axis(session.document(), session.evaluation(), axis)?,
        count: count(session, "count", count_given)?,
        spacing: length(session, "spacing", spacing)?,
        measured: if total {
            LinearSpacing::Total
        } else {
            LinearSpacing::BetweenCopies
        },
        reversed,
    })
}

fn pattern(
    session: &mut Session,
    work: &Work<'_>,
    (body, repeated): (FeatureId, Vec<FeatureId>),
    kind: PatternKind,
    (name, title): (Option<String>, &str),
) -> Result<Answer, ToolError> {
    let document = session.document();
    let details = subject_details(document, body, &repeated);
    let name = feature_name(document, name, title);
    added(
        session,
        work,
        (
            name,
            FeatureKind::from(Pattern::new(body, kind).repeating(repeated)),
        ),
        None,
        details,
    )
}

pub fn linear_pattern(
    session: &mut Session,
    work: &Work<'_>,
    asked: LinearPatterning,
) -> Result<Answer, ToolError> {
    session.evaluate(work)?;
    let chosen = subject(session, asked.body.as_ref(), &asked.features, "repeat")?;
    let first = direction(
        session,
        &asked.axis,
        (&asked.count, &asked.spacing, asked.total, asked.reversed),
    )?;
    let second = asked
        .second
        .as_ref()
        .map(|second| {
            direction(
                session,
                &second.axis,
                (
                    &second.count,
                    &second.spacing,
                    second.total,
                    second.reversed,
                ),
            )
        })
        .transpose()?;
    pattern(
        session,
        work,
        chosen,
        PatternKind::Linear { first, second },
        (asked.name, pattern_tools::LINEAR_TITLE),
    )
}

pub fn circular_pattern(
    session: &mut Session,
    work: &Work<'_>,
    asked: CircularPatterning,
) -> Result<Answer, ToolError> {
    session.evaluate(work)?;
    let chosen = subject(session, asked.body.as_ref(), &asked.features, "repeat")?;
    let axis = places::axis(session.document(), session.evaluation(), &asked.axis)?;
    let angle = match &asked.angle {
        Some(angle) => amount(session, "angle", angle, Dimension::ANGLE, true)?,
        None => solid_tools::degrees(FULL_TURN_DEGREES),
    };
    let kind = PatternKind::Circular(CircularPattern {
        axis: Some(axis),
        count: count(session, "count", &asked.count)?,
        angle,
        reversed: asked.reversed,
    });
    pattern(
        session,
        work,
        chosen,
        kind,
        (asked.name, pattern_tools::CIRCULAR_TITLE),
    )
}

pub fn combine(
    session: &mut Session,
    work: &Work<'_>,
    asked: Combining,
) -> Result<Answer, ToolError> {
    session.evaluate(work)?;
    let document = session.document();
    let evaluation = session.evaluation();
    let body = references::body(document, evaluation, &asked.body)?;
    let mut tools = Vec::with_capacity(asked.tools.len());
    for named in &asked.tools {
        let tool = references::body(document, evaluation, named)?;
        if tool == body {
            return Err(FeatureError::CombinedWithItself {
                body: body_label(document, body),
            }
            .into());
        }
        if !tools.contains(&tool) {
            tools.push(tool);
        }
    }
    let Some((&tool, more_tools)) = tools.split_first() else {
        return Err(FeatureError::NoTools {
            body: body_label(document, body),
        }
        .into());
    };
    let operation = match asked.operation {
        CombineName::Join => CombineOperation::Join,
        CombineName::Cut => CombineOperation::Cut,
        CombineName::Intersect => CombineOperation::Intersect,
    };
    let mut combine = Combine::new(body, tool, operation);
    combine.more_tools = more_tools.to_vec();
    combine.keep_tool = asked.keep_tools;
    let mut details = Map::new();
    details.insert("body".to_owned(), json!(body_label(document, body)));
    let name = feature_name(document, asked.name, combine_tools::TITLE);
    added(
        session,
        work,
        (name, FeatureKind::Combine(combine)),
        None,
        details,
    )
}

pub fn move_body(
    session: &mut Session,
    work: &Work<'_>,
    asked: Moving,
) -> Result<Answer, ToolError> {
    let zero_length = || LengthUnit::Millimetre.default_length(0.0);
    let zero_turn = || solid_tools::degrees(0.0);
    let offset = match &asked.offset {
        Some([x, y, z]) => [
            signed_length(session, "offset", x)?,
            signed_length(session, "offset", y)?,
            signed_length(session, "offset", z)?,
        ],
        None => [zero_length(), zero_length(), zero_length()],
    };
    let turned = match &asked.turn {
        Some([x, y, z]) => [
            turn(session, "turn", x)?,
            turn(session, "turn", y)?,
            turn(session, "turn", z)?,
        ],
        None => [zero_turn(), zero_turn(), zero_turn()],
    };
    let axis_angle = match (&asked.turn_axis, &asked.turn_angle) {
        (Some(_), None) | (None, Some(_)) => return Err(FeatureError::HalfAnAxisTurn.into()),
        (Some(_), Some(_)) if asked.turn.is_some() => return Err(FeatureError::TwoTurns.into()),
        (Some(axis), Some(angle)) => Some((axis, turn(session, "turn_angle", angle)?)),
        (None, None) => None,
    };
    session.evaluate(work)?;
    let document = session.document();
    let evaluation = session.evaluation();
    let body = references::body(document, evaluation, &asked.body)?;
    let about = match axis_angle {
        Some((axis, angle)) => TurnCentre::Axis(Box::new(AxisTurn {
            axis: places::axis(document, evaluation, axis)?,
            angle,
        })),
        None => TurnCentre::Body,
    };
    let movement = Move {
        body,
        offset,
        turn: turned,
        copy: asked.copy,
        about,
        frame: None,
    };
    let title = if asked.copy {
        move_tools::COPY_NAME
    } else {
        move_tools::TITLE
    };
    let mut details = Map::new();
    details.insert("body".to_owned(), json!(body_label(document, body)));
    let name = feature_name(document, asked.name, title);
    added(
        session,
        work,
        (name, FeatureKind::Move(movement)),
        None,
        details,
    )
}

fn datum_added(
    session: &mut Session,
    work: &Work<'_>,
    datum: Datum,
    name: Option<String>,
) -> Result<Answer, ToolError> {
    let name = feature_name(session.document(), name, datum.title());
    added(
        session,
        work,
        (name, FeatureKind::Datum(datum)),
        None,
        Map::new(),
    )
}

pub fn datum_plane(
    session: &mut Session,
    work: &Work<'_>,
    asked: DatumPlaning,
) -> Result<Answer, ToolError> {
    let offset = match &asked.offset {
        Some(offset) => signed_length(session, "offset", offset)?,
        None => LengthUnit::Millimetre.default_length(0.0),
    };
    let angle = asked
        .turn_angle
        .as_ref()
        .map(|angle| turn(session, "turn_angle", angle))
        .transpose()?;
    session.evaluate(work)?;
    let document = session.document();
    let evaluation = session.evaluation();
    let base = places::plane(document, evaluation, &asked.base)?;
    let rotation = match (&asked.turn_axis, angle) {
        (Some(axis), Some(angle)) => Some(PlaneRotation {
            axis: places::axis(document, evaluation, axis)?,
            angle,
        }),
        (None, None) => None,
        (Some(_), None) | (None, Some(_)) => return Err(FeatureError::HalfAnAxisTurn.into()),
    };
    datum_added(
        session,
        work,
        Datum::Plane(DatumPlane {
            base,
            rotation,
            offset,
        }),
        asked.name,
    )
}

pub fn datum_axis(
    session: &mut Session,
    work: &Work<'_>,
    asked: DatumAxising,
) -> Result<Answer, ToolError> {
    session.evaluate(work)?;
    let document = session.document();
    let evaluation = session.evaluation();
    let axis = match (&asked.along, &asked.planes) {
        (Some(_), Some(_)) => return Err(FeatureError::TwoDatumAxes.into()),
        (None, None) => return Err(FeatureError::NoDatumAxis.into()),
        (Some(along), None) => DatumAxis::Along(places::axis(document, evaluation, along)?),
        (None, Some([first, second])) => DatumAxis::Intersection(
            places::plane(document, evaluation, first)?,
            places::plane(document, evaluation, second)?,
        ),
    };
    datum_added(session, work, Datum::Axis(axis), asked.name)
}
