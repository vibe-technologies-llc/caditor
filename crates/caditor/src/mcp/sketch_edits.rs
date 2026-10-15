use std::collections::BTreeSet;

use caditor_document::{Document, Evaluation, FeatureId, Transaction};
use caditor_expression::{Dimension, Expression, Unit};
use caditor_sketch::{
    BlendError, BreakError, Caps, ChamferSize, CircularPattern, ClipError, Corner, Dimensioned,
    EntityId, ExtendError, FilletError, GearError, MAX_LENGTH, MirrorError, OffsetError,
    PatternError, PatternRow, RectangularPattern, Sketch, SketchError, SplitError, Spread,
    TangentError, TrimError,
};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::{
    feature_tree::count,
    field::{self, Expected},
    filleting,
    mcp::{
        references::{self, EntityName, Named},
        session::{Session, Work},
        sketch_additions::{self, Blending, Copying, Gearing, TangentCircling},
        sketch_cuts::{self, Breaking, Constructing, Respacing, Splitting},
        sketch_moves,
        sketch_projections::{self, Intersecting, Projecting},
        tools::{Answer, ToolError},
        writing::{self, MOST_ITEMS, SketchingError, point},
    },
    mirroring, offsetting, patterning,
    projecting::ProjectionError,
    sketch_status::{SketchSummary, up_to_date_result},
    trimming,
    units::{Units, attach_unit},
};

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum SketchEditError {
    #[error("that piece cannot be trimmed: {0}")]
    Trim(TrimError),
    #[error("that end cannot be extended: {0}")]
    Extend(ExtendError),
    #[error(
        "give the end to extend as end (the point ending the curve) or near (a position by that end), not both and not neither"
    )]
    ExtendEnd,
    #[error("the curves cannot be offset: {0}")]
    Offset(OffsetError),
    #[error("the geometry cannot be mirrored: {0}")]
    Mirror(MirrorError),
    #[error("the geometry cannot be patterned: {0}")]
    Pattern(PatternError),
    #[error("that corner cannot be rounded: {0}")]
    Fillet(FilletError),
    #[error("that corner cannot be cut: {0}")]
    Chamfer(FilletError),
    #[error(
        "give a chamfer one distance, and either second_distance (a distance on the second curve) or angle, not both"
    )]
    ChamferForm,
    #[error("{entity} in {sketch} is not a point")]
    NotAPoint { sketch: String, entity: EntityName },
    #[error(
        "give move either by (an offset) or to (where the first point of what moves goes), not both and not neither"
    )]
    MoveTarget,
    #[error(
        "{sketch} has no solved shape to move from: it failed or is waiting; get_sketch says why"
    )]
    NotSolved { sketch: String },
    #[error("{reason}")]
    Unmovable { reason: String },
    #[error("the sketch cannot be solved with that change: {0}")]
    Unsolved(SketchError),
    #[error(
        "nothing moved: the constraints of {sketch} hold it where it is; remove or turn off a constraint first"
    )]
    Held { sketch: String },
    #[error("keep the geometry within {} m of the sketch's origin", MAX_LENGTH / 1_000.0)]
    OutOfReach,
    #[error(
        "{entity} in {sketch} belongs to the geometry being turned or scaled, so it cannot be what it turns or scales about; give a separate point"
    )]
    AboutMoves { sketch: String, entity: EntityName },
    #[error("a scale factor is a finite number above zero, not {0}")]
    Factor(f64),
    #[error("that curve cannot be split: {0}")]
    Split(SplitError),
    #[error("the curves cannot be broken: {0}")]
    Break(BreakError),
    #[error("the splines cannot be respaced: {0}")]
    Respace(SketchError),
    #[error(
        "nothing to change in {sketch}: construction applies to curves, not points, and every curve given already is what was asked"
    )]
    NothingToToggle { sketch: String },
    #[error(
        "none of the splines given in {sketch} is an evenly spaced fit-point spline; only those from files saved before centripetal spacing need respacing"
    )]
    NothingToRespace { sketch: String },
    #[error("the tangent circle cannot be drawn: {0}")]
    TangentCircle(TangentError),
    #[error("the blend cannot be drawn: {0}")]
    Blend(BlendError),
    #[error("the gear cannot be drawn: {0}")]
    Gear(GearError),
    #[error("give the gear's centre as center (a point) or at (a position), not both")]
    GearCentre,
    #[error(
        "{given} is not an ISO 606 roller chain size the app offers; use a designation such as 08B or 10A"
    )]
    UnknownChain { given: String },
    #[error("the geometry cannot be copied: {0}")]
    Clip(ClipError),
    #[error("give copy either by (an offset) or to (where the middle of the copy goes), not both")]
    CopyTarget,
    #[error("{body} is made after {sketch} in the tree, so it cannot be projected into it")]
    LaterBody { body: String, sketch: String },
    #[error(
        "{body} has no {what} {key} where {sketch} sees it; list_edges and list_faces name them"
    )]
    NotSeen {
        body: String,
        what: &'static str,
        key: String,
        sketch: String,
    },
    #[error("edge {key} of {body} cannot be projected into {sketch}")]
    NotProjectable {
        body: String,
        key: String,
        sketch: String,
    },
    #[error(
        "give something to project: edges, faces or corners of a body, a source_sketch, axes, datums or the origin"
    )]
    NothingToProject,
    #[error("edges, faces and corners are named with their body: give body")]
    BodyNeeded,
    #[error("source_curves are curves of a sketch: give source_sketch with them")]
    CurvesWithoutSketch,
    #[error(
        "intersect with one of: a body (optionally with one of its faces), a datum plane, or a principal plane (XY, XZ, YZ)"
    )]
    IntersectWays,
    #[error(transparent)]
    Projection(ProjectionError),
    #[error(
        "everything named is already projected into {sketch}, or every edge of the faces is seen end-on from it"
    )]
    AlreadyProjected { sketch: String },
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum Modification {
    Trim(Trimming),
    Extend(Extending),
    Offset(Offsetting),
    Mirror(Mirroring),
    RectangularPattern(RowPatterning),
    CircularPattern(TurnPatterning),
    Fillet(CornerRounding),
    Chamfer(CornerCutting),
    Move(sketch_moves::Moving),
    Rotate(sketch_moves::Turning),
    Scale(sketch_moves::Scaling),
    Project(Projecting),
    Intersect(Intersecting),
    Construction(Constructing),
    Split(Splitting),
    Break(Breaking),
    Respace(Respacing),
    TangentCircle(TangentCircling),
    Blend(Blending),
    Gear(Gearing),
    Copy(Copying),
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Trimming {
    pub sketch: Named,
    pub pieces: Vec<PieceAt>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PieceAt {
    pub curve: EntityName,
    pub near: [f64; 2],
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Extending {
    pub sketch: Named,
    pub curve: EntityName,
    pub end: Option<EntityName>,
    pub near: Option<[f64; 2]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndCaps {
    Round,
    Flat,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Offsetting {
    pub sketch: Named,
    pub curves: Vec<EntityName>,
    pub distance: String,
    pub toward: Option<[f64; 2]>,
    pub both_sides: Option<EndCaps>,
    #[serde(default)]
    pub whole_chain: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mirroring {
    pub sketch: Named,
    pub items: Vec<EntityName>,
    pub about: EntityName,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Row {
    pub count: usize,
    pub spacing: String,
    pub angle: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RowPatterning {
    pub sketch: Named,
    pub items: Vec<EntityName>,
    pub count: usize,
    pub spacing: String,
    pub angle: Option<String>,
    pub rows: Option<Row>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TurnPatterning {
    pub sketch: Named,
    pub items: Vec<EntityName>,
    pub center: EntityName,
    pub count: usize,
    pub angle: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum CornerName {
    Point(EntityName),
    Between([EntityName; 2]),
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CornerRounding {
    pub sketch: Named,
    pub corners: Vec<CornerName>,
    pub radius: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CornerCutting {
    pub sketch: Named,
    pub corners: Vec<CornerName>,
    pub distance: String,
    pub second_distance: Option<String>,
    pub angle: Option<String>,
}

pub(super) struct Target<'s> {
    pub(super) document: &'s Document,
    pub(super) evaluation: &'s Evaluation,
    pub(super) feature: FeatureId,
    pub(super) name: &'s str,
    pub(super) definition: &'s Sketch,
    pub(super) shown: &'s Sketch,
}

pub(super) struct Planned {
    pub(super) transaction: Transaction,
    pub(super) details: Map<String, Value>,
}

impl Planned {
    pub(super) fn of(transaction: Transaction) -> Self {
        Self {
            transaction,
            details: Map::new(),
        }
    }
}

impl<'s> Target<'s> {
    pub(super) fn of(session: &'s Session, named: &Named) -> Result<Self, ToolError> {
        let document = session.document();
        let evaluation = session.evaluation();
        let (feature, definition) = references::sketch(document, named)?;
        let shown = up_to_date_result(evaluation, feature.id())
            .map_or(definition, |result| &result.geometry);
        Ok(Self {
            document,
            evaluation,
            feature: feature.id(),
            name: &feature.name,
            definition,
            shown,
        })
    }

    pub(super) fn entity(&self, named: &EntityName) -> Result<EntityId, ToolError> {
        Ok(references::entity(self.name, self.shown, named)?)
    }

    pub(super) fn entities(&self, named: &[EntityName]) -> Result<Vec<EntityId>, ToolError> {
        if named.is_empty() {
            return Err(SketchingError::Nothing.into());
        }
        if named.len() > MOST_ITEMS {
            return Err(SketchingError::TooMany.into());
        }
        named.iter().map(|one| self.entity(one)).collect()
    }

    pub(super) fn point_of(&self, named: &EntityName) -> Result<EntityId, ToolError> {
        let id = self.entity(named)?;
        if self.shown.point(id).is_some() {
            Ok(id)
        } else {
            Err(SketchEditError::NotAPoint {
                sketch: self.name.to_owned(),
                entity: named.clone(),
            }
            .into())
        }
    }

    pub(super) fn working(&self) -> Sketch {
        let mut working = self.definition.clone();
        for (id, entity) in self.shown.entities() {
            let same = self
                .definition
                .entity(id)
                .is_some_and(|defined| defined != entity && defined.same_structure(entity));
            if same {
                let _ = working.replace_entity(id, entity.clone());
            }
        }
        working
    }

    pub(super) fn reshaped(
        &self,
        label: String,
        change: impl FnOnce(&mut Sketch) -> Result<(), ToolError>,
    ) -> Result<Transaction, ToolError> {
        let before = self.working();
        let mut after = before.clone();
        change(&mut after)?;
        let mut transaction = self.document.transaction(label);
        writing::settled(&mut transaction, self.evaluation, self.feature);
        transaction.reshape_sketch(self.feature, &before, &after);
        Ok(transaction.finish())
    }
}

pub(super) fn amount(
    session: &Session,
    field_name: &'static str,
    text: &str,
    dimension: Dimension,
) -> Result<Dimensioned, ToolError> {
    let parameters = session.parameters();
    let refused = |reason: String| ToolError::Value {
        field: field_name,
        text: text.to_owned(),
        reason,
    };
    let expected = Expected {
        dimension: Some(dimension),
        non_negative: false,
    };
    let parsed = field::parse_expression(
        session.document(),
        &parameters,
        text,
        expected,
        Units::default(),
    )
    .map_err(refused)?;
    let plain_angle = dimension == Dimension::ANGLE
        && parameters
            .evaluate_expression(&parsed)
            .is_ok_and(|found| found.dimension.is_plain());
    let expression = if plain_angle {
        attach_unit(parsed, Unit::Degree)
    } else {
        parsed
    };
    let value = expression
        .evaluate_as(dimension, &|id| parameters.value(id))
        .map_err(|error| refused(field::sentence(&error.to_string())))?;
    Ok(Dimensioned { expression, value })
}

fn entity_ids(sketch: &Sketch) -> BTreeSet<u64> {
    sketch.entities().map(|(id, _)| id.raw()).collect()
}

fn listed(ids: impl Iterator<Item = u64>) -> Value {
    Value::Array(ids.map(|id| json!(id)).collect())
}

pub fn modify(
    session: &mut Session,
    work: &Work<'_>,
    asked: Modification,
) -> Result<Answer, ToolError> {
    session.evaluate(work)?;
    let (feature, before, planned) = {
        let target = Target::of(session, sketch_of(&asked))?;
        let planned = plan(session, work, &target, &asked)?;
        (target.feature, entity_ids(target.definition), planned)
    };
    let answer = writing::apply(session, work, planned.transaction, planned.details)?;
    let mut data = answer.data;
    if let Some(after) = session
        .document()
        .feature(feature)
        .and_then(|found| found.kind.sketch())
        .map(entity_ids)
    {
        data.insert(
            "added_entities".to_owned(),
            listed(after.difference(&before).copied()),
        );
        data.insert(
            "removed_entities".to_owned(),
            listed(before.difference(&after).copied()),
        );
    }
    data.insert(
        "sketch_status".to_owned(),
        json!(SketchSummary::of(session.evaluation(), feature).status_text()),
    );
    Ok(Answer::of(data))
}

fn sketch_of(asked: &Modification) -> &Named {
    match asked {
        Modification::Trim(asked) => &asked.sketch,
        Modification::Extend(asked) => &asked.sketch,
        Modification::Offset(asked) => &asked.sketch,
        Modification::Mirror(asked) => &asked.sketch,
        Modification::RectangularPattern(asked) => &asked.sketch,
        Modification::CircularPattern(asked) => &asked.sketch,
        Modification::Fillet(asked) => &asked.sketch,
        Modification::Chamfer(asked) => &asked.sketch,
        Modification::Move(asked) => &asked.sketch,
        Modification::Rotate(asked) => &asked.sketch,
        Modification::Scale(asked) => &asked.sketch,
        Modification::Project(asked) => &asked.sketch,
        Modification::Intersect(asked) => &asked.sketch,
        Modification::Construction(asked) => &asked.sketch,
        Modification::Split(asked) => &asked.sketch,
        Modification::Break(asked) => &asked.sketch,
        Modification::Respace(asked) => &asked.sketch,
        Modification::TangentCircle(asked) => &asked.sketch,
        Modification::Blend(asked) => &asked.sketch,
        Modification::Gear(asked) => &asked.sketch,
        Modification::Copy(asked) => &asked.sketch,
    }
}

fn plan(
    session: &Session,
    work: &Work<'_>,
    target: &Target<'_>,
    asked: &Modification,
) -> Result<Planned, ToolError> {
    match asked {
        Modification::Trim(asked) => trim(target, asked),
        Modification::Extend(asked) => extend(target, asked),
        Modification::Offset(asked) => offset(session, target, asked),
        Modification::Mirror(asked) => mirror(target, asked),
        Modification::RectangularPattern(asked) => rectangular_pattern(session, target, asked),
        Modification::CircularPattern(asked) => circular_pattern(session, target, asked),
        Modification::Fillet(asked) => fillet(session, target, asked),
        Modification::Chamfer(asked) => chamfer(session, target, asked),
        Modification::Move(asked) => sketch_moves::move_geometry(session, work, target, asked),
        Modification::Rotate(asked) => sketch_moves::rotate(session, work, target, asked),
        Modification::Scale(asked) => sketch_moves::scale(session, work, target, asked),
        Modification::Project(asked) => sketch_projections::project(target, asked),
        Modification::Intersect(asked) => sketch_projections::intersect(target, asked),
        Modification::Construction(asked) => sketch_cuts::construction(target, asked),
        Modification::Split(asked) => sketch_cuts::split(target, asked),
        Modification::Break(asked) => sketch_cuts::break_curves(target, asked),
        Modification::Respace(asked) => sketch_cuts::respace(target, asked),
        Modification::TangentCircle(asked) => {
            sketch_additions::tangent_circle(session, target, asked)
        }
        Modification::Blend(asked) => sketch_additions::blend(target, asked),
        Modification::Gear(asked) => sketch_additions::gear(session, target, asked),
        Modification::Copy(asked) => sketch_additions::copy(target, asked),
    }
}

fn trim(target: &Target<'_>, asked: &Trimming) -> Result<Planned, ToolError> {
    if asked.pieces.is_empty() {
        return Err(SketchingError::Nothing.into());
    }
    if asked.pieces.len() > MOST_ITEMS {
        return Err(SketchingError::TooMany.into());
    }
    let working = target.working();
    let pieces = asked
        .pieces
        .iter()
        .map(|piece| {
            let curve = target.entity(&piece.curve)?;
            working
                .trim_piece(curve, point(piece.near)?)
                .map_err(|error| ToolError::from(SketchEditError::Trim(error)))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let label = match pieces.as_slice() {
        [only] => format!("Trim {}", working.entity_label(only.curve)),
        pieces => format!("Trim {}", count(pieces.len(), "piece", "pieces")),
    };
    let transaction = target.reshaped(label, |sketch| {
        for piece in &pieces {
            let middle = piece.middle();
            if let Some(curve) = trimming::carrier(sketch, piece.curve, middle) {
                sketch.trim(curve, middle).map_err(SketchEditError::Trim)?;
            }
        }
        Ok(())
    })?;
    Ok(Planned::of(transaction))
}

fn extend(target: &Target<'_>, asked: &Extending) -> Result<Planned, ToolError> {
    let curve = target.entity(&asked.curve)?;
    let near = match (&asked.end, asked.near) {
        (Some(end), None) => {
            let end = target.point_of(end)?;
            target.shown.point(end).ok_or(SketchEditError::ExtendEnd)?
        }
        (None, Some(near)) => point(near)?,
        _ => return Err(SketchEditError::ExtendEnd.into()),
    };
    let working = target.working();
    let extension = working
        .extension(curve, near)
        .map_err(SketchEditError::Extend)?;
    let label = format!("Extend {}", working.entity_label(curve));
    let transaction = target.reshaped(label, |sketch| {
        sketch
            .extend(curve, near)
            .map(|_| ())
            .map_err(|error| SketchEditError::Extend(error).into())
    })?;
    let mut planned = Planned::of(transaction);
    planned.details.insert(
        "extended_to".to_owned(),
        json!(working.entity_label(extension.target)),
    );
    Ok(planned)
}

fn offset(
    session: &Session,
    target: &Target<'_>,
    asked: &Offsetting,
) -> Result<Planned, ToolError> {
    let picked = target.entities(&asked.curves)?;
    let distance = amount(session, "distance", &asked.distance, Dimension::LENGTH)?;
    let working = target.working();
    let curves: Vec<EntityId> = if asked.whole_chain {
        let chained: BTreeSet<EntityId> = picked
            .iter()
            .flat_map(|curve| working.offset_chain_through(*curve))
            .collect();
        chained.into_iter().collect()
    } else {
        picked
    };
    let chain = working
        .offset_chain(&curves)
        .map_err(SketchEditError::Offset)?;
    let side = match asked.toward {
        Some(toward) => chain.side_of(point(toward)?),
        None => chain.default_side(),
    };
    let (side, millimetres, expression) = if distance.value < 0.0 {
        let expression = match distance.expression {
            Expression::Negate(inner) => *inner,
            other => Expression::Negate(Box::new(other)),
        };
        (side.other(), -distance.value, expression)
    } else {
        (side, distance.value, distance.expression)
    };
    let caps = asked.both_sides.map(|caps| match caps {
        EndCaps::Round => Caps::Round,
        EndCaps::Flat => Caps::Flat,
    });
    let label = match caps {
        Some(_) => offsetting::BOTH_TRANSACTION,
        None => offsetting::TRANSACTION,
    };
    let transaction = target.reshaped(label.to_owned(), |sketch| {
        match caps {
            Some(caps) => sketch.offset_both_sides(&curves, millimetres, expression, caps),
            None => sketch.offset(&curves, side, millimetres, expression),
        }
        .map(|_| ())
        .map_err(|error| SketchEditError::Offset(error).into())
    })?;
    let mut planned = Planned::of(transaction);
    planned
        .details
        .insert("offset_curves".to_owned(), writing::ids(&curves));
    Ok(planned)
}

fn mirror(target: &Target<'_>, asked: &Mirroring) -> Result<Planned, ToolError> {
    let items = target.entities(&asked.items)?;
    let about = target.entity(&asked.about)?;
    let transaction = target.reshaped(mirroring::TRANSACTION.to_owned(), |sketch| {
        sketch
            .mirror(&items, about)
            .map(|_| ())
            .map_err(|error| SketchEditError::Mirror(error).into())
    })?;
    Ok(Planned::of(transaction))
}

fn angle_or(
    session: &Session,
    text: Option<&String>,
    otherwise: impl FnOnce() -> Dimensioned,
) -> Result<Dimensioned, ToolError> {
    match text {
        Some(text) => amount(session, "angle", text, Dimension::ANGLE),
        None => Ok(otherwise()),
    }
}

fn rectangular_pattern(
    session: &Session,
    target: &Target<'_>,
    asked: &RowPatterning,
) -> Result<Planned, ToolError> {
    let items = target.entities(&asked.items)?;
    let first = PatternRow {
        count: asked.count,
        spacing: amount(session, "spacing", &asked.spacing, Dimension::LENGTH)?,
        angle: angle_or(session, asked.angle.as_ref(), || patterning::turned(0.0))?,
    };
    let second = asked
        .rows
        .as_ref()
        .map(|row| {
            Ok::<_, ToolError>(PatternRow {
                count: row.count,
                spacing: amount(session, "spacing", &row.spacing, Dimension::LENGTH)?,
                angle: angle_or(session, row.angle.as_ref(), || {
                    patterning::square_to(&first.angle)
                })?,
            })
        })
        .transpose()?;
    let pattern = RectangularPattern { first, second };
    let transaction = target.reshaped(patterning::TRANSACTION.to_owned(), |sketch| {
        sketch
            .rectangular_pattern(&items, &pattern)
            .map(|_| ())
            .map_err(|error| SketchEditError::Pattern(error).into())
    })?;
    Ok(Planned::of(transaction))
}

fn circular_pattern(
    session: &Session,
    target: &Target<'_>,
    asked: &TurnPatterning,
) -> Result<Planned, ToolError> {
    let items = target.entities(&asked.items)?;
    let centre = target.entity(&asked.center)?;
    let spread = match &asked.angle {
        Some(text) => Spread::Total(amount(session, "angle", text, Dimension::ANGLE)?),
        None => Spread::FullTurn,
    };
    let pattern = CircularPattern {
        count: asked.count,
        spread,
    };
    let transaction = target.reshaped(patterning::TRANSACTION.to_owned(), |sketch| {
        sketch
            .circular_pattern(&items, centre, &pattern)
            .map(|_| ())
            .map_err(|error| SketchEditError::Pattern(error).into())
    })?;
    Ok(Planned::of(transaction))
}

fn corners(
    target: &Target<'_>,
    named: &[CornerName],
    refused: fn(FilletError) -> SketchEditError,
) -> Result<Vec<Corner>, ToolError> {
    if named.is_empty() {
        return Err(SketchingError::Nothing.into());
    }
    if named.len() > MOST_ITEMS {
        return Err(SketchingError::TooMany.into());
    }
    let working = target.working();
    named
        .iter()
        .map(|corner| {
            let found = match corner {
                CornerName::Point(point) => working.corner_at(target.entity(point)?),
                CornerName::Between([first, second]) => {
                    working.corner_between(target.entity(first)?, target.entity(second)?)
                }
            };
            found.map_err(|error| refused(error).into())
        })
        .collect()
}

fn corner_label(corners: &[Corner], one: &str, several: &str) -> String {
    match corners {
        [_] => one.to_owned(),
        _ => several.to_owned(),
    }
}

fn fillet(
    session: &Session,
    target: &Target<'_>,
    asked: &CornerRounding,
) -> Result<Planned, ToolError> {
    let corners = corners(target, &asked.corners, SketchEditError::Fillet)?;
    let radius = amount(session, "radius", &asked.radius, Dimension::LENGTH)?;
    let label = corner_label(
        &corners,
        filleting::TRANSACTION,
        filleting::CORNERS_TRANSACTION,
    );
    let transaction = target.reshaped(label, |sketch| {
        for corner in &corners {
            let current = sketch
                .corner_at(corner.point)
                .map_err(SketchEditError::Fillet)?;
            sketch
                .fillet(&current, radius.value, radius.expression.clone())
                .map_err(SketchEditError::Fillet)?;
        }
        Ok(())
    })?;
    Ok(Planned::of(transaction))
}

fn chamfer(
    session: &Session,
    target: &Target<'_>,
    asked: &CornerCutting,
) -> Result<Planned, ToolError> {
    let corners = corners(target, &asked.corners, SketchEditError::Chamfer)?;
    let distance = amount(session, "distance", &asked.distance, Dimension::LENGTH)?;
    let size = match (&asked.second_distance, &asked.angle) {
        (None, None) => ChamferSize::Equal(distance),
        (Some(second), None) => ChamferSize::Distances {
            first: distance,
            second: amount(session, "second_distance", second, Dimension::LENGTH)?,
        },
        (None, Some(angle)) => ChamferSize::DistanceAndAngle {
            distance,
            angle: amount(session, "angle", angle, Dimension::ANGLE)?,
        },
        (Some(_), Some(_)) => return Err(SketchEditError::ChamferForm.into()),
    };
    let label = corner_label(
        &corners,
        filleting::CHAMFER_TRANSACTION,
        filleting::CHAMFER_CORNERS_TRANSACTION,
    );
    let transaction = target.reshaped(label, |sketch| {
        for corner in &corners {
            let current = sketch
                .corner_at(corner.point)
                .map_err(SketchEditError::Chamfer)?;
            sketch
                .chamfer(&current, &size)
                .map_err(SketchEditError::Chamfer)?;
        }
        Ok(())
    })?;
    Ok(Planned::of(transaction))
}
