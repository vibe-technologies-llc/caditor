use std::{collections::BTreeSet, fmt};

use caditor_document::{
    AxisTurn, Blend, BlendKind, BodyOperation, ChamferForm, CircularPattern, Combine,
    CombineOperation, Document, EdgeGroup, Edit, Evaluation, FaceThickness, FeatureId, FeatureKind,
    LinearDirection, LinearSpacing, Mirror, Move, OffsetFace, Pattern, PatternKind, PlaneReference,
    Primitive, PrimitiveAnchor, PrimitiveKind, RegionChoice, Revolve, RevolveAxis, RevolveExtent,
    Shell, ShellSide, SketchRegion, SolidFeature, TurnCentre, repeatable_on,
};
use caditor_expression::{Dimension, Expression};
use caditor_geometry::Point2;
use caditor_kernel::{EdgeNaming, EdgeReference, FaceReference};
use caditor_sketch::{EntityId, Sketch};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::{
    combine_tools, editing,
    field::{self, Expected},
    mcp::{
        places::{self, AxisChoice, PlaneChoice},
        references::{self, EntityName, Named, body_label},
        session::{Session, Work},
        solids::{WallName, wall_of},
        tools::{Answer, ToolError},
        writing::{ModellingError, OperationName, apply, operation_words, with_feature},
    },
    mirror_tools, move_tools, offset_face_tools, pattern_tools, primitive_tools, scene, selection,
    shell_tools,
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
    pub sketch: Option<Named>,
    pub axis: Option<RevolveAxisChoice>,
    pub angle: Option<Amount>,
    pub second_angle: Option<Amount>,
    pub up_to: Option<PlaneChoice>,
    pub symmetric: Option<bool>,
    pub reversed: Option<bool>,
    pub regions: Option<RegionsAsk>,
    pub thin_wall: Option<WallAsk>,
    pub wall_side: Option<WallName>,
    pub operation: Option<OperationName>,
    pub body: Option<Named>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AllName {
    All,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum RegionsAsk {
    All(AllName),
    Inside(Vec<[f64; 2]>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OffName {
    Off,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum WallAsk {
    Off(OffName),
    Thickness(Amount),
}

impl WallAsk {
    pub fn thickness(&self) -> Option<&Amount> {
        match self {
            Self::Off(_) => None,
            Self::Thickness(thickness) => Some(thickness),
        }
    }

    pub fn is_off(&self) -> bool {
        matches!(self, Self::Off(_))
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Filleting {
    pub body: Option<Named>,
    pub edges: Option<Vec<String>>,
    pub faces: Option<Vec<String>>,
    pub all_edges: Option<bool>,
    pub radius: Option<Amount>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Chamfering {
    pub body: Option<Named>,
    pub edges: Option<Vec<String>>,
    pub faces: Option<Vec<String>>,
    pub all_edges: Option<bool>,
    pub distance: Option<Amount>,
    pub second_distance: Option<Amount>,
    pub angle: Option<Amount>,
    pub flipped: Option<bool>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SideName {
    Inside,
    Outside,
    BothSides,
}

impl SideName {
    fn side(self) -> ShellSide {
        match self {
            Self::Inside => ShellSide::Inside,
            Self::Outside => ShellSide::Outside,
            Self::BothSides => ShellSide::BothSides,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FaceWall {
    pub face: String,
    pub thickness: Amount,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Shelling {
    pub body: Option<Named>,
    pub open_faces: Option<Vec<String>>,
    pub thickness: Option<Amount>,
    pub face_thicknesses: Option<Vec<FaceWall>>,
    pub side: Option<SideName>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FaceOffset {
    pub body: Option<Named>,
    pub faces: Option<Vec<String>>,
    pub distance: Option<Amount>,
    pub tangent: Option<bool>,
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
}

fn sizes_of(kind: PrimitiveKind) -> &'static [&'static str] {
    match kind {
        PrimitiveKind::Box => &["length", "width", "height"],
        PrimitiveKind::Cylinder => &["diameter", "height"],
        PrimitiveKind::Sphere => &["diameter"],
        PrimitiveKind::Torus => &["diameter", "tube"],
        PrimitiveKind::Cone => &["bottom", "top", "height"],
        PrimitiveKind::Wedge => &["length", "width", "height", "top_length"],
        PrimitiveKind::Prism => &["sides", "diameter", "height"],
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
    pub kind: Option<PrimitiveName>,
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
    pub reversed: Option<bool>,
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
    pub features: Option<Vec<Named>>,
    pub faces: Option<Vec<String>>,
    pub plane: Option<PlaneChoice>,
    pub keep_original: Option<bool>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum CopyName {
    InRow(u32),
    InGrid([u32; 2]),
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Direction {
    pub axis: Option<AxisChoice>,
    pub count: Option<Amount>,
    pub spacing: Option<Amount>,
    pub total: Option<bool>,
    pub reversed: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinearPatterning {
    pub body: Option<Named>,
    pub features: Option<Vec<Named>>,
    pub axis: Option<AxisChoice>,
    pub count: Option<Amount>,
    pub spacing: Option<Amount>,
    pub total: Option<bool>,
    pub reversed: Option<bool>,
    pub second: Option<Direction>,
    pub skipped: Option<Vec<CopyName>>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CircularPatterning {
    pub body: Option<Named>,
    pub features: Option<Vec<Named>>,
    pub axis: Option<AxisChoice>,
    pub count: Option<Amount>,
    pub angle: Option<Amount>,
    pub reversed: Option<bool>,
    pub skipped: Option<Vec<u32>>,
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
    pub body: Option<Named>,
    pub tools: Option<Vec<Named>>,
    pub operation: Option<CombineName>,
    pub keep_tools: Option<bool>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Moving {
    pub body: Option<Named>,
    pub offset: Option<[Amount; 3]>,
    pub turn: Option<[Amount; 3]>,
    pub turn_axis: Option<AxisChoice>,
    pub turn_angle: Option<Amount>,
    pub copy: Option<bool>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum FeatureError {
    #[error("give {field}")]
    Missing { field: &'static str },
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
    #[error("{second} goes with {first}, which sets the first side")]
    SecondWithoutFirst {
        second: &'static str,
        first: &'static str,
    },
    #[error("a symmetric {feature} spreads one {size} evenly, so it takes no {second}")]
    SymmetricTwoSides {
        feature: &'static str,
        size: &'static str,
        second: &'static str,
    },
    #[error("mirror the faces or the features, not both")]
    FacesAndFeatures,
    #[error("{sketch} has no closed region to choose")]
    NoRegions { sketch: String },
    #[error(
        "{sketch} has no region around [{x}, {y}]; get_sketch lists a point inside each region"
    )]
    NoRegionAt { sketch: String, x: f64, y: f64 },
    #[error("regions takes a point inside each region wanted, such as [[5, 5]], or \"all\"")]
    NoRegionPoints,
    #[error("a revolve ends up_to a plane or turns by an angle, not both")]
    UpToWithAngle,
    #[error("the original stays; skipped names copies from 1 on (or [column, row] in a grid)")]
    SkippedOriginal,
}

pub struct Built {
    pub kind: FeatureKind,
    pub title: &'static str,
    pub name: Option<String>,
    pub hide: Vec<FeatureId>,
    pub details: Map<String, Value>,
}

impl Built {
    pub(super) fn new(kind: FeatureKind, title: &'static str, name: Option<String>) -> Self {
        Self {
            kind,
            title,
            name,
            hide: Vec::new(),
            details: Map::new(),
        }
    }

    pub(super) fn hiding(mut self, sketch: FeatureId) -> Self {
        self.hide.push(sketch);
        self
    }

    pub(super) fn telling(mut self, key: &str, value: Value) -> Self {
        self.details.insert(key.to_owned(), value);
        self
    }
}

pub(super) fn needed<T>(value: Option<T>, field: &'static str) -> Result<T, ToolError> {
    value.ok_or_else(|| FeatureError::Missing { field }.into())
}

pub(super) fn amount(
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
        reason: Box::new(reason.into()),
    })
}

pub(super) fn length(
    session: &Session,
    field_name: &'static str,
    given: &Amount,
) -> Result<Expression, ToolError> {
    amount(session, field_name, given, Dimension::LENGTH, true)
}

pub(super) fn signed_length(
    session: &Session,
    field_name: &'static str,
    given: &Amount,
) -> Result<Expression, ToolError> {
    amount(session, field_name, given, Dimension::LENGTH, false)
}

pub(super) fn turn(
    session: &Session,
    field_name: &'static str,
    given: &Amount,
) -> Result<Expression, ToolError> {
    amount(session, field_name, given, Dimension::ANGLE, false)
}

pub(super) fn count(
    session: &Session,
    field_name: &'static str,
    given: &Amount,
) -> Result<Expression, ToolError> {
    amount(session, field_name, given, Dimension::NONE, true)
}

pub(super) fn parsed(
    session: &Session,
    given: Option<&Amount>,
    parse: impl Fn(&Session, &Amount) -> Result<Expression, ToolError>,
) -> Result<Option<Expression>, ToolError> {
    given.map(|given| parse(session, given)).transpose()
}

pub(super) fn body_of(
    session: &Session,
    asked: Option<&Named>,
    kept: Option<FeatureId>,
) -> Result<FeatureId, ToolError> {
    match asked {
        Some(named) => Ok(references::body(
            session.document(),
            session.evaluation(),
            named,
        )?),
        None => needed(kept, "body"),
    }
}

pub(super) fn sketch_of<'d>(
    document: &'d Document,
    asked: Option<&Named>,
    kept: Option<FeatureId>,
) -> Result<(FeatureId, &'d str, &'d Sketch), ToolError> {
    let named = match (asked, kept) {
        (Some(named), _) => named.clone(),
        (None, Some(kept)) => Named::Id(kept.raw()),
        (None, None) => return Err(FeatureError::Missing { field: "sketch" }.into()),
    };
    let (feature, sketch) = references::sketch(document, &named)?;
    Ok((feature.id(), feature.name.as_str(), sketch))
}

pub(super) fn regions_chosen(
    evaluation: &Evaluation,
    (sketch, name): (FeatureId, &str),
    asked: Option<&RegionsAsk>,
    kept: RegionChoice,
) -> Result<RegionChoice, FeatureError> {
    let points = match asked {
        None => return Ok(kept),
        Some(RegionsAsk::All(_)) => return Ok(RegionChoice::All),
        Some(RegionsAsk::Inside(points)) if points.is_empty() => {
            return Err(FeatureError::NoRegionPoints);
        }
        Some(RegionsAsk::Inside(points)) => points,
    };
    let regions = sketch_regions_of(evaluation, (sketch, name))?;
    let keys = points
        .iter()
        .map(|at| region_at(regions, name, *at).map(|region| region.region.key()))
        .collect::<Result<BTreeSet<_>, _>>()?;
    Ok(RegionChoice::Chosen(scene::region_references(
        &keys, regions,
    )))
}

pub(super) fn sketch_regions_of<'e>(
    evaluation: &'e Evaluation,
    (sketch, name): (FeatureId, &str),
) -> Result<&'e [SketchRegion], FeatureError> {
    selection::sketch_regions(evaluation, sketch).ok_or_else(|| FeatureError::NoRegions {
        sketch: name.to_owned(),
    })
}

pub(super) fn region_at<'e>(
    regions: &'e [SketchRegion],
    sketch: &str,
    [x, y]: [f64; 2],
) -> Result<&'e SketchRegion, FeatureError> {
    regions
        .iter()
        .find(|region| region.region.contains(Point2::new(x, y)))
        .ok_or_else(|| FeatureError::NoRegionAt {
            sketch: sketch.to_owned(),
            x,
            y,
        })
}

pub(super) fn added(
    session: &mut Session,
    work: &Work<'_>,
    built: Built,
) -> Result<Answer, ToolError> {
    let document = session.document();
    let name = built
        .name
        .unwrap_or_else(|| editing::next_feature_name(document, built.title));
    let mut transaction = document.transaction(format!("Create {name}"));
    let feature = transaction.add_feature(name, built.kind);
    for sketch in built.hide {
        if visibility::is_shown(document, sketch) {
            transaction.edit(Edit::SetFeatureHidden {
                id: sketch,
                hidden: true,
            });
        }
    }
    let transaction = transaction.finish();
    let answer = apply(session, work, transaction, built.details)?;
    Ok(with_feature(session, answer, feature))
}

pub(super) fn chosen_operation(
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

pub(super) fn operation_asked(
    session: &Session,
    asked: (Option<OperationName>, Option<&Named>),
    kept: Option<BodyOperation>,
) -> Result<Option<BodyOperation>, ToolError> {
    let document = session.document();
    let chosen = asked
        .1
        .map(|body| references::body(document, session.evaluation(), body))
        .transpose()?;
    Ok(match (asked.0, chosen) {
        (Some(name), chosen) => Some(chosen_operation(
            document,
            name,
            chosen.or_else(|| kept.and_then(BodyOperation::target)),
        )?),
        (None, Some(body)) => Some(match kept {
            Some(BodyOperation::Remove(_)) => BodyOperation::Remove(body),
            Some(BodyOperation::Intersect(_)) => BodyOperation::Intersect(body),
            Some(BodyOperation::Add(_) | BodyOperation::NewBody) | None => BodyOperation::Add(body),
        }),
        (None, None) => kept,
    })
}

fn operation_details(document: &Document, operation: BodyOperation) -> Value {
    json!(operation_words(document, operation))
}

struct RevolveSides {
    angle: Option<Expression>,
    second: Option<Expression>,
    symmetric: bool,
    reversed: bool,
}

impl RevolveSides {
    fn of(extent: &RevolveExtent) -> Self {
        let (angle, second, symmetric, reversed) = match extent {
            RevolveExtent::Full => (None, None, false, false),
            RevolveExtent::OneSide { angle, reversed } => {
                (Some(angle.clone()), None, false, *reversed)
            }
            RevolveExtent::Symmetric { angle } => (Some(angle.clone()), None, true, false),
            RevolveExtent::TwoSides { forward, backward } => {
                (Some(forward.clone()), Some(backward.clone()), false, false)
            }
            RevolveExtent::UpTo { reversed, .. } => (None, None, false, *reversed),
        };
        Self {
            angle,
            second,
            symmetric,
            reversed,
        }
    }

    fn extent(self) -> Result<RevolveExtent, FeatureError> {
        match (self.angle, self.second, self.symmetric) {
            (_, Some(_), true) => Err(FeatureError::SymmetricTwoSides {
                feature: "revolve",
                size: "angle",
                second: "second_angle",
            }),
            (None, Some(_), false) => Err(FeatureError::SecondWithoutFirst {
                second: "second_angle",
                first: "angle",
            }),
            (None, None, _) => Ok(RevolveExtent::Full),
            (Some(angle), None, true) => Ok(RevolveExtent::Symmetric { angle }),
            (Some(angle), None, false) => Ok(RevolveExtent::OneSide {
                angle,
                reversed: self.reversed,
            }),
            (Some(forward), Some(backward), false) => {
                Ok(RevolveExtent::TwoSides { forward, backward })
            }
        }
    }
}

pub fn revolve_built(
    session: &mut Session,
    work: &Work<'_>,
    asked: Revolution,
    base: Option<&Revolve>,
) -> Result<Built, ToolError> {
    let angle = parsed(session, asked.angle.as_ref(), |session, angle| {
        turn(session, "angle", angle)
    })?;
    let second = parsed(session, asked.second_angle.as_ref(), |session, angle| {
        turn(session, "second_angle", angle)
    })?;
    let thickness = parsed(
        session,
        asked.thin_wall.as_ref().and_then(WallAsk::thickness),
        |session, wall| length(session, "thin_wall", wall),
    )?;
    session.evaluate(work)?;
    let values = session.parameters();
    let document = session.document();
    let evaluation = session.evaluation();
    let (sketch, sketch_name, definition) = sketch_of(
        document,
        asked.sketch.as_ref(),
        base.map(|base| base.sketch),
    )?;
    let axis = match (&asked.axis, base) {
        (None, Some(base)) => base.axis.clone(),
        (None, None) => RevolveAxis::Sketch(EntityId::VERTICAL_AXIS),
        (Some(RevolveAxisChoice::Line(line)), _) => {
            RevolveAxis::Sketch(references::entity(sketch_name, definition, line)?)
        }
        (Some(RevolveAxisChoice::Model(choice)), _) => {
            RevolveAxis::Model(places::axis(document, evaluation, choice)?)
        }
    };
    let turns = angle.is_some() || second.is_some() || asked.symmetric.is_some();
    if asked.up_to.is_some() && turns {
        return Err(FeatureError::UpToWithAngle.into());
    }
    let kept_end = base.and_then(|base| match &base.extent {
        RevolveExtent::UpTo { target, reversed } => Some((target.clone(), *reversed)),
        _ => None,
    });
    let extent = match (&asked.up_to, kept_end, base) {
        (Some(choice), kept, _) => RevolveExtent::up_to(
            places::plane(document, evaluation, choice)?,
            asked
                .reversed
                .unwrap_or(kept.is_some_and(|(_, reversed)| reversed)),
        ),
        (None, Some((target, reversed)), _) if !turns => RevolveExtent::UpTo {
            target,
            reversed: asked.reversed.unwrap_or(reversed),
        },
        (None, _, Some(base)) if !turns && asked.reversed.is_none() => base.extent.clone(),
        _ => {
            let kept = base.map_or(
                RevolveSides {
                    angle: None,
                    second: None,
                    symmetric: false,
                    reversed: false,
                },
                |base| RevolveSides::of(&base.extent),
            );
            let symmetric = asked.symmetric.unwrap_or(kept.symmetric);
            RevolveSides {
                angle: angle.or(kept.angle),
                second: second.or(kept.second.filter(|_| !symmetric)),
                symmetric,
                reversed: asked.reversed.unwrap_or(kept.reversed),
            }
            .extent()?
        }
    };
    let regions = regions_chosen(
        evaluation,
        (sketch, sketch_name),
        asked.regions.as_ref(),
        match base {
            Some(base) if base.sketch == sketch => base.regions.clone(),
            Some(_) | None => RegionChoice::All,
        },
    )?;
    let wall = wall_of(
        thickness,
        asked.wall_side,
        base.and_then(|base| base.wall.as_deref()),
        asked.thin_wall.as_ref().is_some_and(WallAsk::is_off),
    )?;
    let revolve = match base {
        Some(base) => Revolve {
            sketch,
            regions,
            axis,
            extent,
            wall,
            ..base.clone()
        },
        None => Revolve {
            sketch,
            regions,
            axis,
            extent,
            operation: BodyOperation::NewBody,
            start: None,
            other_bodies: Vec::new(),
            side: None,
            wall,
        },
    };
    let asked_operation = operation_asked(
        session,
        (asked.operation, asked.body.as_ref()),
        base.map(|base| base.operation),
    )?;
    let solid = SolidFeature::Revolve(revolve);
    let operation = asked_operation.unwrap_or_else(|| {
        solid_tools::default_operation(
            document,
            Reach {
                evaluation,
                parameters: &values,
            },
            &solid,
        )
    });
    let solid = solid_tools::with_operation(&solid, operation);
    Ok(Built::new(
        FeatureKind::Solid(solid),
        solid_tools::Sweep::Revolve.label(),
        asked.name,
    )
    .hiding(sketch)
    .telling("operation", operation_details(document, operation)))
}

pub fn revolve(
    session: &mut Session,
    work: &Work<'_>,
    asked: Revolution,
) -> Result<Answer, ToolError> {
    let built = revolve_built(session, work, asked, None)?;
    added(session, work, built)
}

struct EdgeChoice<'a> {
    body: Option<&'a Named>,
    edges: Option<&'a [String]>,
    faces: Option<&'a [String]>,
    all_edges: Option<bool>,
}

impl EdgeChoice<'_> {
    fn is_given(&self) -> bool {
        self.edges.is_some() || self.faces.is_some() || self.all_edges.is_some()
    }
}

struct BlendSize {
    field: &'static str,
    size: Option<Expression>,
    form: Option<ChamferForm>,
    flipped: Option<bool>,
}

fn captured_edges(
    session: &Session,
    body: FeatureId,
    choice: &EdgeChoice<'_>,
    verb: &'static str,
) -> Result<(Vec<EdgeReference>, Vec<EdgeGroup>), ToolError> {
    let edge_keys = choice.edges.unwrap_or_default();
    let face_keys = choice.faces.unwrap_or_default();
    let all_edges = choice.all_edges.unwrap_or(false);
    if all_edges && !(edge_keys.is_empty() && face_keys.is_empty()) {
        return Err(FeatureError::AllEdgesAndMore.into());
    }
    if !all_edges && edge_keys.is_empty() && face_keys.is_empty() {
        return Err(FeatureError::NoEdges { verb }.into());
    }
    let edges = edge_references(session, body, edge_keys)?;
    let mut groups: Vec<EdgeGroup> = face_references(session, body, face_keys)?
        .into_iter()
        .map(EdgeGroup::Face)
        .collect();
    if all_edges {
        groups.push(EdgeGroup::Body);
    }
    Ok((edges, groups))
}

pub(super) fn edge_references(
    session: &Session,
    body: FeatureId,
    keys: &[String],
) -> Result<Vec<EdgeReference>, ToolError> {
    let document = session.document();
    let evaluation = session.evaluation();
    keys.iter()
        .map(|key| {
            let (_, edge, shown) = places::edge(document, evaluation, body, key)?;
            let naming = EdgeNaming::new(&shown.solid);
            EdgeReference::capture_in(&naming, edge).ok_or_else(|| {
                places::PlaceError::NoEdge {
                    body: body_label(document, body),
                    edge: key.clone(),
                }
                .into()
            })
        })
        .collect()
}

fn blend(
    session: &mut Session,
    work: &Work<'_>,
    kind: BlendKind,
    (choice, size): (EdgeChoice<'_>, BlendSize),
    (name, base): (Option<String>, Option<&Blend>),
) -> Result<Built, ToolError> {
    let verb = kind.noun();
    if base.is_none() && !choice.is_given() {
        return Err(FeatureError::NoEdges { verb }.into());
    }
    session.evaluate(work)?;
    let body = body_of(session, choice.body, base.map(|base| base.body))?;
    let (edges, groups) = match base {
        Some(base) if !choice.is_given() => (base.edges.clone(), base.groups.clone()),
        Some(_) | None => captured_edges(session, body, &choice, verb)?,
    };
    let blend = Blend {
        kind,
        body,
        edges,
        groups,
        size: needed(
            size.size.or_else(|| base.map(|base| base.size.clone())),
            size.field,
        )?,
        form: size
            .form
            .or_else(|| base.map(|base| base.form.clone()))
            .unwrap_or(ChamferForm::Equal),
        flipped: size
            .flipped
            .or_else(|| base.map(|base| base.flipped))
            .unwrap_or(false),
    };
    let label = body_label(session.document(), body);
    Ok(Built::new(FeatureKind::Blend(blend), kind.title(), name).telling("body", json!(label)))
}

pub fn fillet_built(
    session: &mut Session,
    work: &Work<'_>,
    asked: Filleting,
    base: Option<&Blend>,
) -> Result<Built, ToolError> {
    let radius = parsed(session, asked.radius.as_ref(), |session, radius| {
        length(session, "radius", radius)
    })?;
    blend(
        session,
        work,
        BlendKind::Fillet,
        (
            EdgeChoice {
                body: asked.body.as_ref(),
                edges: asked.edges.as_deref(),
                faces: asked.faces.as_deref(),
                all_edges: asked.all_edges,
            },
            BlendSize {
                field: "radius",
                size: radius,
                form: None,
                flipped: None,
            },
        ),
        (asked.name, base),
    )
}

pub fn fillet(
    session: &mut Session,
    work: &Work<'_>,
    asked: Filleting,
) -> Result<Answer, ToolError> {
    let built = fillet_built(session, work, asked, None)?;
    added(session, work, built)
}

pub fn chamfer_built(
    session: &mut Session,
    work: &Work<'_>,
    asked: Chamfering,
    base: Option<&Blend>,
) -> Result<Built, ToolError> {
    let distance = parsed(session, asked.distance.as_ref(), |session, distance| {
        length(session, "distance", distance)
    })?;
    let form = match (&asked.second_distance, &asked.angle) {
        (Some(_), Some(_)) => return Err(FeatureError::TwoChamferForms.into()),
        (Some(second), None) => Some(ChamferForm::TwoDistances {
            second: length(session, "second_distance", second)?,
        }),
        (None, Some(angle)) => Some(ChamferForm::DistanceAngle {
            angle: amount(session, "angle", angle, Dimension::ANGLE, true)?,
        }),
        (None, None) => None,
    };
    blend(
        session,
        work,
        BlendKind::Chamfer,
        (
            EdgeChoice {
                body: asked.body.as_ref(),
                edges: asked.edges.as_deref(),
                faces: asked.faces.as_deref(),
                all_edges: asked.all_edges,
            },
            BlendSize {
                field: "distance",
                size: distance,
                form,
                flipped: asked.flipped,
            },
        ),
        (asked.name, base),
    )
}

pub fn chamfer(
    session: &mut Session,
    work: &Work<'_>,
    asked: Chamfering,
) -> Result<Answer, ToolError> {
    let built = chamfer_built(session, work, asked, None)?;
    added(session, work, built)
}

pub(super) fn face_references(
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

pub fn shell_built(
    session: &mut Session,
    work: &Work<'_>,
    asked: Shelling,
    base: Option<&Shell>,
) -> Result<Built, ToolError> {
    let thickness = parsed(session, asked.thickness.as_ref(), |session, thickness| {
        length(session, "thickness", thickness)
    })?;
    let wall_thicknesses = asked
        .face_thicknesses
        .as_ref()
        .map(|walls| {
            walls
                .iter()
                .map(|wall| length(session, "face_thicknesses thickness", &wall.thickness))
                .collect::<Result<Vec<_>, ToolError>>()
        })
        .transpose()?;
    session.evaluate(work)?;
    let body = body_of(session, asked.body.as_ref(), base.map(|base| base.body))?;
    let open = match (&asked.open_faces, base) {
        (Some(keys), _) => face_references(session, body, keys)?,
        (None, Some(base)) => base.open.clone(),
        (None, None) => Vec::new(),
    };
    let faces = match (&asked.face_thicknesses, wall_thicknesses) {
        (Some(walls), Some(thicknesses)) => walls
            .iter()
            .zip(thicknesses)
            .map(|(wall, thickness)| {
                let face = face_references(session, body, std::slice::from_ref(&wall.face))?
                    .into_iter()
                    .next();
                face.map(|face| FaceThickness { face, thickness })
                    .ok_or_else(|| {
                        ToolError::from(references::ReferenceError::NoFace {
                            body: body_label(session.document(), body),
                            face: wall.face.clone(),
                        })
                    })
            })
            .collect::<Result<Vec<_>, ToolError>>()?,
        _ => base.map(|base| base.faces.clone()).unwrap_or_default(),
    };
    let shell = Shell {
        body,
        open,
        thickness: needed(
            thickness.or_else(|| base.map(|base| base.thickness.clone())),
            "thickness",
        )?,
        side: asked
            .side
            .map(SideName::side)
            .or_else(|| base.map(|base| base.side))
            .unwrap_or(ShellSide::Inside),
        faces,
    };
    let label = body_label(session.document(), body);
    Ok(
        Built::new(FeatureKind::Shell(shell), shell_tools::TITLE, asked.name)
            .telling("body", json!(label)),
    )
}

pub fn shell(session: &mut Session, work: &Work<'_>, asked: Shelling) -> Result<Answer, ToolError> {
    let built = shell_built(session, work, asked, None)?;
    added(session, work, built)
}

pub fn offset_face_built(
    session: &mut Session,
    work: &Work<'_>,
    asked: FaceOffset,
    base: Option<&OffsetFace>,
) -> Result<Built, ToolError> {
    if asked.faces.as_ref().is_some_and(Vec::is_empty) || (asked.faces.is_none() && base.is_none())
    {
        return Err(FeatureError::NoFacesToOffset.into());
    }
    let distance = parsed(session, asked.distance.as_ref(), |session, distance| {
        signed_length(session, "distance", distance)
    })?;
    session.evaluate(work)?;
    let body = body_of(session, asked.body.as_ref(), base.map(|base| base.body))?;
    let faces = match (&asked.faces, base) {
        (Some(keys), _) => face_references(session, body, keys)?,
        (None, Some(base)) => base.faces.clone(),
        (None, None) => Vec::new(),
    };
    let offset = OffsetFace {
        body,
        faces,
        distance: needed(
            distance.or_else(|| base.map(|base| base.distance.clone())),
            "distance",
        )?,
        tangent: asked
            .tangent
            .or_else(|| base.map(|base| base.tangent))
            .unwrap_or(false),
    };
    let label = body_label(session.document(), body);
    Ok(Built::new(
        FeatureKind::OffsetFace(offset),
        offset_face_tools::TITLE,
        asked.name,
    )
    .telling("body", json!(label)))
}

pub fn offset_face(
    session: &mut Session,
    work: &Work<'_>,
    asked: FaceOffset,
) -> Result<Answer, ToolError> {
    let built = offset_face_built(session, work, asked, None)?;
    added(session, work, built)
}

pub fn primitive_built(
    session: &mut Session,
    work: &Work<'_>,
    asked: NewPrimitive,
    base: Option<&Primitive>,
) -> Result<Built, ToolError> {
    let kind = match (asked.kind, base) {
        (Some(name), _) => name.kind(),
        (None, Some(base)) => base.shape.kind(),
        (None, None) => return Err(FeatureError::Missing { field: "kind" }.into()),
    };
    let mut shape = match base {
        Some(base) if base.shape.kind() == kind => base.shape.clone(),
        Some(_) | None => primitive_tools::default_shape(kind, LengthUnit::Millimetre),
    };
    let names = sizes_of(kind);
    for (size, given) in asked.given_sizes() {
        let Some(position) = names.iter().position(|name| *name == size) else {
            return Err(FeatureError::SizeNotTaken {
                kind: kind.name(),
                sizes: names.join(", "),
                size,
            }
            .into());
        };
        let value = if size == "sides" {
            count(session, size, given)?
        } else {
            length(session, size, given)?
        };
        if let Some(slot) = shape.sizes_mut().into_iter().nth(position) {
            *slot = value;
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
    let (plane, middle) = match (&asked.plane, base) {
        (None, Some(base)) => (base.plane.clone(), None),
        (None, None) => (
            PlaneReference::Principal(caditor_document::PrincipalPlane::Xy),
            Some(Point2::ZERO),
        ),
        (Some(PlaneChoice::Face(on)), _) => {
            let placed = places::on_face(document, evaluation, on)?;
            (
                placed.reference,
                Some(placed.middle.unwrap_or(Point2::ZERO)),
            )
        }
        (Some(choice), _) => (
            places::plane(document, evaluation, choice)?,
            Some(Point2::ZERO),
        ),
    };
    let face_body = plane.body();
    let middle_at = |spot: Point2| {
        [
            LengthUnit::Millimetre.measured(spot.x),
            LengthUnit::Millimetre.measured(spot.y),
        ]
    };
    let at = match (at, middle, base) {
        (Some(at), _, _) => at,
        (None, Some(spot), _) => middle_at(spot),
        (None, None, Some(base)) => base.at.clone(),
        (None, None, None) => middle_at(Point2::ZERO),
    };
    let chosen = asked
        .body
        .as_ref()
        .map(|body| references::body(document, evaluation, body))
        .transpose()?;
    let kept = base.map(|base| base.operation);
    let operation = match (asked.operation, chosen, kept) {
        (Some(name), chosen, kept) => chosen_operation(
            document,
            name,
            chosen
                .or(face_body)
                .or_else(|| kept.and_then(BodyOperation::target)),
        )?,
        (None, Some(body), _) => BodyOperation::Add(body),
        (None, None, Some(kept)) => kept,
        (None, None, None) => face_body.map_or(BodyOperation::NewBody, BodyOperation::Add),
    };
    let cutting_into_its_face = face_body.is_some()
        && matches!(
            operation,
            BodyOperation::Remove(_) | BodyOperation::Intersect(_)
        );
    let reversed = match (asked.reversed, base) {
        (Some(reversed), _) => reversed != cutting_into_its_face,
        (None, Some(base)) => base.reversed,
        (None, None) => cutting_into_its_face,
    };
    let anchor = match (asked.anchor, base) {
        (Some(AnchorName::Corner), _) => PrimitiveAnchor::Corner,
        (Some(AnchorName::BaseCentre), _) => PrimitiveAnchor::BaseCentre,
        (Some(AnchorName::Centre), _) => PrimitiveAnchor::Centre,
        (None, Some(base)) if base.shape.kind() == kind => base.anchor,
        (None, Some(_) | None) => primitive_tools::default_anchor(kind),
    };
    let primitive = Primitive {
        shape,
        plane,
        at,
        anchor,
        reversed,
        operation,
    };
    Ok(
        Built::new(FeatureKind::Primitive(primitive), kind.name(), asked.name)
            .telling("operation", operation_details(document, operation)),
    )
}

pub fn primitive(
    session: &mut Session,
    work: &Work<'_>,
    asked: NewPrimitive,
) -> Result<Answer, ToolError> {
    let built = primitive_built(session, work, asked, None)?;
    added(session, work, built)
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

pub(super) fn kept_subject(
    session: &Session,
    (body, features): (Option<&Named>, Option<&[Named]>),
    kept: Option<(FeatureId, &[FeatureId])>,
    verb: &'static str,
) -> Result<(FeatureId, Vec<FeatureId>), ToolError> {
    match (body, features, kept) {
        (None, None, Some((body, repeated))) => Ok((body, repeated.to_vec())),
        (body, features, _) => subject(session, body, features.unwrap_or_default(), verb),
    }
}

fn subject_details(document: &Document, body: FeatureId, repeated: &[FeatureId]) -> Value {
    json!(pattern_tools::subject(document, body, repeated))
}

pub fn mirror_built(
    session: &mut Session,
    work: &Work<'_>,
    asked: Mirroring,
    base: Option<&Mirror>,
) -> Result<Built, ToolError> {
    session.evaluate(work)?;
    let faced = asked.faces.as_ref().is_some_and(|faces| !faces.is_empty());
    let (body, mirrored) = if faced {
        if asked
            .features
            .as_ref()
            .is_some_and(|features| !features.is_empty())
        {
            return Err(FeatureError::FacesAndFeatures.into());
        }
        (
            body_of(session, asked.body.as_ref(), base.map(|base| base.body))?,
            Vec::new(),
        )
    } else {
        kept_subject(
            session,
            (asked.body.as_ref(), asked.features.as_deref()),
            base.map(|base| (base.body, base.mirrored.as_slice())),
            "mirror",
        )?
    };
    let faces = match (&asked.faces, base) {
        (Some(keys), _) => face_references(session, body, keys)?,
        (None, Some(base)) if base.body == body && base.mirrored == mirrored => base.faces.clone(),
        (None, _) => Vec::new(),
    };
    let document = session.document();
    let plane = match &asked.plane {
        Some(choice) => places::plane(document, session.evaluation(), choice)?,
        None => needed(base.map(|base| base.plane.clone()), "plane")?,
    };
    let mut mirror = match base {
        Some(base) if base.body == body && base.mirrored == mirrored => Mirror {
            plane,
            ..base.clone()
        },
        Some(base) => Mirror {
            keep_original: base.keep_original,
            ..Mirror::new(body, plane).mirroring(mirrored.clone())
        },
        None => Mirror::new(body, plane).mirroring(mirrored.clone()),
    };
    mirror.faces = faces;
    if let Some(keep) = asked.keep_original {
        mirror.keep_original = keep;
    }
    let title = if !mirrored.is_empty() {
        mirror_tools::FEATURES_TITLE
    } else if mirror.faces.is_empty() {
        mirror_tools::TITLE
    } else {
        mirror_tools::FACES_TITLE
    };
    let repeats = subject_details(document, body, &mirrored);
    Ok(Built::new(FeatureKind::Mirror(mirror), title, asked.name).telling("repeats", repeats))
}

pub fn mirror(
    session: &mut Session,
    work: &Work<'_>,
    asked: Mirroring,
) -> Result<Answer, ToolError> {
    let built = mirror_built(session, work, asked, None)?;
    added(session, work, built)
}

struct DirectionWords {
    axis: &'static str,
    count: &'static str,
    spacing: &'static str,
}

const FIRST_DIRECTION: DirectionWords = DirectionWords {
    axis: "axis",
    count: "count",
    spacing: "spacing",
};

const SECOND_DIRECTION: DirectionWords = DirectionWords {
    axis: "second.axis",
    count: "second.count",
    spacing: "second.spacing",
};

fn direction(
    session: &Session,
    asked: &Direction,
    base: Option<&LinearDirection>,
    words: &DirectionWords,
) -> Result<LinearDirection, ToolError> {
    let axis = match &asked.axis {
        Some(choice) => places::axis(session.document(), session.evaluation(), choice)?,
        None => needed(base.map(|base| base.axis.clone()), words.axis)?,
    };
    let copies = match &asked.count {
        Some(given) => count(session, words.count, given)?,
        None => needed(base.map(|base| base.count.clone()), words.count)?,
    };
    let spacing = match &asked.spacing {
        Some(given) => length(session, words.spacing, given)?,
        None => needed(base.map(|base| base.spacing.clone()), words.spacing)?,
    };
    let measured = match asked.total {
        Some(true) => LinearSpacing::Total,
        Some(false) => LinearSpacing::BetweenCopies,
        None => base.map_or(LinearSpacing::BetweenCopies, |base| base.measured),
    };
    Ok(LinearDirection {
        axis,
        count: copies,
        spacing,
        measured,
        reversed: asked
            .reversed
            .or_else(|| base.map(|base| base.reversed))
            .unwrap_or(false),
    })
}

pub(super) fn skipped_copies(
    asked: Option<&[CopyName]>,
    base: Option<&Pattern>,
) -> Result<BTreeSet<[u32; 2]>, ToolError> {
    let Some(asked) = asked else {
        return Ok(base.map(|base| base.skipped.clone()).unwrap_or_default());
    };
    asked
        .iter()
        .map(|copy| match *copy {
            CopyName::InRow(0) | CopyName::InGrid([0, 0]) => {
                Err(FeatureError::SkippedOriginal.into())
            }
            CopyName::InRow(index) => Ok([index, 0]),
            CopyName::InGrid(instance) => Ok(instance),
        })
        .collect()
}

pub(super) fn pattern_built(
    session: &Session,
    (body, repeated): (FeatureId, Vec<FeatureId>),
    (kind, skipped): (PatternKind, BTreeSet<[u32; 2]>),
    (name, title): (Option<String>, &'static str),
) -> Built {
    let repeats = subject_details(session.document(), body, &repeated);
    let mut pattern = Pattern::new(body, kind).repeating(repeated);
    pattern.skipped = skipped;
    Built::new(FeatureKind::from(pattern), title, name).telling("repeats", repeats)
}

pub fn linear_pattern_built(
    session: &mut Session,
    work: &Work<'_>,
    asked: LinearPatterning,
    base: Option<&Pattern>,
) -> Result<Built, ToolError> {
    session.evaluate(work)?;
    let chosen = kept_subject(
        session,
        (asked.body.as_ref(), asked.features.as_deref()),
        base.map(|base| (base.body, base.repeated.as_slice())),
        "repeat",
    )?;
    let (base_first, base_second) = match base.map(|base| &base.kind) {
        Some(PatternKind::Linear { first, second }) => (Some(first), second.as_ref()),
        Some(_) | None => (None, None),
    };
    let first = direction(
        session,
        &Direction {
            axis: asked.axis.clone(),
            count: asked.count.clone(),
            spacing: asked.spacing.clone(),
            total: asked.total,
            reversed: asked.reversed,
        },
        base_first,
        &FIRST_DIRECTION,
    )?;
    let second = match &asked.second {
        Some(second) => Some(direction(session, second, base_second, &SECOND_DIRECTION)?),
        None => base_second.cloned(),
    };
    let skipped = skipped_copies(asked.skipped.as_deref(), base)?;
    Ok(pattern_built(
        session,
        chosen,
        (PatternKind::Linear { first, second }, skipped),
        (asked.name, pattern_tools::LINEAR_TITLE),
    ))
}

pub fn linear_pattern(
    session: &mut Session,
    work: &Work<'_>,
    asked: LinearPatterning,
) -> Result<Answer, ToolError> {
    let built = linear_pattern_built(session, work, asked, None)?;
    added(session, work, built)
}

pub fn circular_pattern_built(
    session: &mut Session,
    work: &Work<'_>,
    asked: CircularPatterning,
    base: Option<&Pattern>,
) -> Result<Built, ToolError> {
    session.evaluate(work)?;
    let chosen = kept_subject(
        session,
        (asked.body.as_ref(), asked.features.as_deref()),
        base.map(|base| (base.body, base.repeated.as_slice())),
        "repeat",
    )?;
    let kept = match base.map(|base| &base.kind) {
        Some(PatternKind::Circular(circular)) => Some(circular),
        Some(_) | None => None,
    };
    let axis = match &asked.axis {
        Some(choice) => Some(places::axis(
            session.document(),
            session.evaluation(),
            choice,
        )?),
        None => needed(kept.map(|kept| kept.axis.clone()), "axis")?,
    };
    let angle = match (&asked.angle, kept) {
        (Some(angle), _) => amount(session, "angle", angle, Dimension::ANGLE, true)?,
        (None, Some(kept)) => kept.angle.clone(),
        (None, None) => solid_tools::degrees(FULL_TURN_DEGREES),
    };
    let copies = match &asked.count {
        Some(given) => count(session, "count", given)?,
        None => needed(kept.map(|kept| kept.count.clone()), "count")?,
    };
    let kind = PatternKind::Circular(CircularPattern {
        axis,
        count: copies,
        angle,
        reversed: asked
            .reversed
            .or_else(|| kept.map(|kept| kept.reversed))
            .unwrap_or(false),
    });
    let in_row: Option<Vec<CopyName>> = asked
        .skipped
        .as_ref()
        .map(|copies| copies.iter().copied().map(CopyName::InRow).collect());
    let skipped = skipped_copies(in_row.as_deref(), base)?;
    Ok(pattern_built(
        session,
        chosen,
        (kind, skipped),
        (asked.name, pattern_tools::CIRCULAR_TITLE),
    ))
}

pub fn circular_pattern(
    session: &mut Session,
    work: &Work<'_>,
    asked: CircularPatterning,
) -> Result<Answer, ToolError> {
    let built = circular_pattern_built(session, work, asked, None)?;
    added(session, work, built)
}

pub fn combine_built(
    session: &mut Session,
    work: &Work<'_>,
    asked: Combining,
    base: Option<&Combine>,
) -> Result<Built, ToolError> {
    session.evaluate(work)?;
    let body = body_of(session, asked.body.as_ref(), base.map(|base| base.body))?;
    let document = session.document();
    let evaluation = session.evaluation();
    let mut tools = Vec::new();
    match (&asked.tools, base) {
        (Some(named), _) => {
            for named in named {
                let tool = references::body(document, evaluation, named)?;
                if !tools.contains(&tool) {
                    tools.push(tool);
                }
            }
        }
        (None, Some(base)) => {
            tools.push(base.tool);
            tools.extend(base.more_tools.iter().copied());
        }
        (None, None) => return Err(FeatureError::Missing { field: "tools" }.into()),
    }
    if tools.contains(&body) {
        return Err(FeatureError::CombinedWithItself {
            body: body_label(document, body),
        }
        .into());
    }
    let Some((&tool, more_tools)) = tools.split_first() else {
        return Err(FeatureError::NoTools {
            body: body_label(document, body),
        }
        .into());
    };
    let operation = match (asked.operation, base) {
        (Some(CombineName::Join), _) => CombineOperation::Join,
        (Some(CombineName::Cut), _) => CombineOperation::Cut,
        (Some(CombineName::Intersect), _) => CombineOperation::Intersect,
        (None, Some(base)) => base.operation,
        (None, None) => return Err(FeatureError::Missing { field: "operation" }.into()),
    };
    let mut combine = match base {
        Some(base) => Combine {
            body,
            tool,
            operation,
            ..base.clone()
        },
        None => Combine::new(body, tool, operation),
    };
    combine.more_tools = more_tools.to_vec();
    if let Some(keep) = asked.keep_tools {
        combine.keep_tool = keep;
    }
    Ok(Built::new(
        FeatureKind::Combine(combine),
        combine_tools::TITLE,
        asked.name,
    )
    .telling("body", json!(body_label(document, body))))
}

pub fn combine(
    session: &mut Session,
    work: &Work<'_>,
    asked: Combining,
) -> Result<Answer, ToolError> {
    let built = combine_built(session, work, asked, None)?;
    added(session, work, built)
}

pub(super) fn triple(
    session: &Session,
    field_name: &'static str,
    given: Option<&[Amount; 3]>,
    parse: fn(&Session, &'static str, &Amount) -> Result<Expression, ToolError>,
) -> Result<Option<[Expression; 3]>, ToolError> {
    match given {
        Some([x, y, z]) => Ok(Some([
            parse(session, field_name, x)?,
            parse(session, field_name, y)?,
            parse(session, field_name, z)?,
        ])),
        None => Ok(None),
    }
}

pub fn move_built(
    session: &mut Session,
    work: &Work<'_>,
    asked: Moving,
    base: Option<&Move>,
) -> Result<Built, ToolError> {
    let zero_length = || LengthUnit::Millimetre.default_length(0.0);
    let zero_turn = || solid_tools::degrees(0.0);
    let offset = triple(session, "offset", asked.offset.as_ref(), signed_length)?;
    let turned = triple(session, "turn", asked.turn.as_ref(), turn)?;
    let turns_about_axis = asked.turn_axis.is_some() || asked.turn_angle.is_some();
    if turned.is_some() && turns_about_axis {
        return Err(FeatureError::TwoTurns.into());
    }
    let angle = parsed(session, asked.turn_angle.as_ref(), |session, angle| {
        turn(session, "turn_angle", angle)
    })?;
    session.evaluate(work)?;
    let body = body_of(session, asked.body.as_ref(), base.map(|base| base.body))?;
    let document = session.document();
    let kept_axis = match base.map(|base| &base.about) {
        Some(TurnCentre::Axis(axis)) => Some(axis.as_ref()),
        Some(_) | None => None,
    };
    let zero_turns = || [zero_turn(), zero_turn(), zero_turn()];
    let (turned, about) = match (turned, turns_about_axis, base) {
        (Some(turned), _, _) => (turned, TurnCentre::Body),
        (None, true, _) => {
            let axis = match &asked.turn_axis {
                Some(choice) => Some(places::axis(document, session.evaluation(), choice)?),
                None => kept_axis.map(|kept| kept.axis.clone()),
            };
            let angle = angle.or_else(|| kept_axis.map(|kept| kept.angle.clone()));
            let (Some(axis), Some(angle)) = (axis, angle) else {
                return Err(FeatureError::HalfAnAxisTurn.into());
            };
            (
                zero_turns(),
                TurnCentre::Axis(Box::new(AxisTurn { axis, angle })),
            )
        }
        (None, false, Some(base)) => (base.turn.clone(), base.about.clone()),
        (None, false, None) => (zero_turns(), TurnCentre::Body),
    };
    let copy = asked
        .copy
        .or_else(|| base.map(|base| base.copy))
        .unwrap_or(false);
    let movement = Move {
        body,
        offset: offset
            .or_else(|| base.map(|base| base.offset.clone()))
            .unwrap_or_else(|| [zero_length(), zero_length(), zero_length()]),
        turn: turned,
        copy,
        about,
        frame: base.and_then(|base| base.frame),
    };
    let title = if copy {
        move_tools::COPY_NAME
    } else {
        move_tools::TITLE
    };
    Ok(Built::new(FeatureKind::Move(movement), title, asked.name)
        .telling("body", json!(body_label(document, body))))
}

pub fn move_body(
    session: &mut Session,
    work: &Work<'_>,
    asked: Moving,
) -> Result<Answer, ToolError> {
    let built = move_built(session, work, asked, None)?;
    added(session, work, built)
}
