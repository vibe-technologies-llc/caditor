use std::{collections::BTreeSet, time::Instant};

use caditor_document::{
    BodyOperation, Document, Edit, Evaluation, Extrude, ExtrudeEnd, ExtrudeExtent, FeatureId,
    FeatureKind, HoleDepth, PrincipalPlane, RegionChoice, SketchFeature, SolidFeature, Transaction,
    TransactionBuilder, hole_centres,
};
use caditor_expression::{Dimension, Expression, check_name};
use caditor_geometry::Point2;
use caditor_kernel::MAX_SIZE;
use caditor_sketch::{Constraint, ConstraintId, Entity, EntityId, Sketch};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::{
    constraint_trial::{self, ConstraintTrial, PATIENCE},
    editing, extrusion_detection,
    field::{self, Expected},
    hole_tools,
    last_values::Starts,
    mcp::{
        reading::{feature_brief, plane, problems},
        references::{self, EntityName, Named, body_label},
        session::{Change, Performed, Session, Work},
        tools::{Answer, ToolError},
    },
    sketch_placement::{self, FaceChoice},
    sketch_status::up_to_date_result,
    solid_tools::{self, Reach, Sweep},
    units::{LengthUnit, Units},
    visibility,
};

pub const MOST_ITEMS: usize = 200;
const THROUGH_ALL: [&str; 3] = ["through all", "through_all", "through"];

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParameterValue {
    pub name: String,
    pub expression: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum PlaneName {
    #[serde(rename = "XY")]
    Xy,
    #[serde(rename = "XZ")]
    Xz,
    #[serde(rename = "YZ")]
    Yz,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewSketch {
    pub plane: Option<PlaneName>,
    pub body: Option<Named>,
    pub face: Option<String>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Drawing {
    pub sketch: Named,
    pub items: Vec<Item>,
    #[serde(default)]
    pub construction: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Item {
    Point {
        at: [f64; 2],
    },
    Line {
        from: [f64; 2],
        to: [f64; 2],
    },
    Polyline {
        points: Vec<[f64; 2]>,
        #[serde(default)]
        closed: bool,
    },
    Rectangle {
        corner: [f64; 2],
        opposite: [f64; 2],
    },
    Circle {
        center: [f64; 2],
        radius: f64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Constraining {
    pub sketch: Named,
    pub constraints: Vec<ConstraintItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ConstraintItem {
    Coincident {
        a: EntityName,
        b: EntityName,
    },
    Horizontal {
        line: EntityName,
    },
    Vertical {
        line: EntityName,
    },
    Parallel {
        a: EntityName,
        b: EntityName,
    },
    Perpendicular {
        a: EntityName,
        b: EntityName,
    },
    Equal {
        a: EntityName,
        b: EntityName,
    },
    Tangent {
        a: EntityName,
        b: EntityName,
    },
    Concentric {
        a: EntityName,
        b: EntityName,
    },
    Collinear {
        a: EntityName,
        b: EntityName,
    },
    Midpoint {
        point: EntityName,
        curve: EntityName,
    },
    Symmetric {
        a: EntityName,
        b: EntityName,
        about: EntityName,
    },
    Fix {
        point: EntityName,
    },
    Distance {
        from: EntityName,
        to: EntityName,
        value: String,
    },
    HorizontalDistance {
        from: EntityName,
        to: EntityName,
        value: String,
    },
    VerticalDistance {
        from: EntityName,
        to: EntityName,
        value: String,
    },
    Radius {
        curve: EntityName,
        value: String,
    },
    Diameter {
        curve: EntityName,
        value: String,
    },
    Angle {
        from: EntityName,
        to: EntityName,
        value: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationName {
    NewBody,
    Add,
    Remove,
    Intersect,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Extrusion {
    pub sketch: Named,
    pub distance: Option<String>,
    #[serde(default)]
    pub through_all: bool,
    #[serde(default)]
    pub symmetric: bool,
    pub operation: Option<OperationName>,
    pub body: Option<Named>,
    pub reversed: Option<bool>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Drilling {
    pub sketch: Named,
    pub diameter: String,
    pub depth: String,
    pub body: Option<Named>,
    #[serde(default)]
    pub reversed: bool,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum SketchingError {
    #[error("say where the sketch goes: a plane (XY, XZ or YZ), or a body and one of its faces")]
    NoPlace,
    #[error("a sketch goes on a plane or on a face, not both")]
    BothPlaces,
    #[error("a face is named with its body: give both body and face")]
    FaceWithoutBody,
    #[error("the sketch cannot go on that face: {reason}")]
    UnusableFace { reason: &'static str },
    #[error("{0} is not a finite number of millimetres within a kilometre of the sketch origin")]
    Coordinate(f64),
    #[error("a circle's radius must be a finite number above zero, not {0}")]
    Radius(f64),
    #[error("a polyline needs at least two points")]
    ShortPolyline,
    #[error("a line needs two different ends")]
    ZeroLine,
    #[error("a rectangle needs corners apart in both directions")]
    FlatRectangle,
    #[error("a call draws or constrains at most {MOST_ITEMS} items")]
    TooMany,
    #[error("there is nothing to draw or constrain; give at least one item")]
    Nothing,
    #[error("{entity} in {sketch} is not a point, so it cannot be fixed")]
    NotAPoint { sketch: String, entity: EntityName },
    #[error("{reason}")]
    Conflicting { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ModellingError {
    #[error("give the extrusion a distance, or through_all for a cut")]
    NoDistance,
    #[error("give the extrusion a distance or through_all, not both")]
    DistanceAndThroughAll,
    #[error("through_all is for an extrusion that removes or intersects; give a distance")]
    ThroughAllAdds,
    #[error("a symmetric extrusion takes a distance, not through_all")]
    SymmetricThroughAll,
    #[error("{operation} needs the body it changes, and the model has none yet; give body")]
    NoTarget { operation: &'static str },
    #[error(
        "{sketch} has no free point or circle to drill at; add points with add_sketch_geometry"
    )]
    NothingToDrill { sketch: String },
    #[error("there is no body to drill; make one first or give body")]
    NoBodyToDrill,
}

fn value(
    session: &Session,
    field_name: &'static str,
    text: &str,
    dimension: Dimension,
) -> Result<Expression, ToolError> {
    let expected = Expected {
        dimension: Some(dimension),
        non_negative: true,
    };
    field::parse_expression(
        session.document(),
        &session.parameters(),
        text,
        expected,
        Units::default(),
    )
    .map_err(|reason| ToolError::Value {
        field: field_name,
        text: text.to_owned(),
        reason,
    })
}

fn failing_ids(session: &Session) -> Option<BTreeSet<u64>> {
    session.is_evaluated().then(|| {
        problems(session.document(), session.evaluation())
            .into_iter()
            .map(|(id, _, _)| id)
            .collect()
    })
}

fn apply(
    session: &mut Session,
    work: &Work<'_>,
    transaction: Transaction,
    details: Map<String, Value>,
) -> Result<Answer, ToolError> {
    let before = failing_ids(session);
    let performed = session.perform(Change::Apply(transaction))?;
    report(session, work, &performed, before.as_ref(), details)
}

fn report(
    session: &mut Session,
    work: &Work<'_>,
    performed: &Performed,
    before: Option<&BTreeSet<u64>>,
    mut details: Map<String, Value>,
) -> Result<Answer, ToolError> {
    session.evaluate(work)?;
    let failing: Vec<Value> = problems(session.document(), session.evaluation())
        .into_iter()
        .map(|(id, name, words)| {
            let mut entry = Map::new();
            entry.insert("id".to_owned(), json!(id));
            entry.insert("name".to_owned(), json!(name));
            words.into_json(&mut entry);
            if let Some(before) = before {
                entry.insert("new".to_owned(), json!(!before.contains(&id)));
            }
            Value::Object(entry)
        })
        .collect();
    details.insert("change".to_owned(), json!(performed.label));
    if !performed.changed {
        details.insert(
            "changed".to_owned(),
            json!("nothing: the model already was that way"),
        );
    }
    details.insert("failing".to_owned(), Value::Array(failing));
    details.insert("unsaved_changes".to_owned(), json!(session.is_unsaved()));
    Ok(Answer::of(details))
}

fn settled(transaction: &mut TransactionBuilder<'_>, evaluation: &Evaluation, sketch: FeatureId) {
    if let Some(result) = up_to_date_result(evaluation, sketch) {
        transaction.settle_sketch(sketch, &result.geometry);
    }
}

pub fn set_parameter(
    session: &mut Session,
    work: &Work<'_>,
    asked: ParameterValue,
) -> Result<Answer, ToolError> {
    let name = asked.name.trim();
    let values = session.parameters();
    let document = session.document();
    let existing = document.parameter_named(name);
    let current = existing
        .and_then(|parameter| values.get(parameter.id()))
        .and_then(|value| value.as_ref().ok())
        .map(|value| value.dimension);
    let expression = field::parameter_expression(
        document,
        &values,
        &asked.expression,
        current,
        Units::default(),
    )
    .map_err(|reason| ToolError::Value {
        field: "expression",
        text: asked.expression.clone(),
        reason,
    })?;
    let transaction = match existing {
        Some(parameter) => Transaction::single(
            format!("Edit {}", parameter.name),
            Edit::SetParameterExpression {
                id: parameter.id(),
                expression,
            },
        ),
        None => {
            check_name(name).map_err(|error| ToolError::Value {
                field: "name",
                text: name.to_owned(),
                reason: error.to_string(),
            })?;
            let mut transaction = document.transaction(format!("Add {name}"));
            transaction.add_parameter(name, expression);
            transaction.finish()
        }
    };
    let mut details = Map::new();
    details.insert("parameter".to_owned(), json!(name));
    let answer = apply(session, work, transaction, details)?;
    let mut data = answer.data;
    let values = session.parameters();
    if let Some(parameter) = session.document().parameter_named(name) {
        let shown = match values.get(parameter.id()) {
            Some(Ok(value)) => value.to_string(),
            Some(Err(error)) => format!("cannot be worked out: {error}"),
            None => String::new(),
        };
        data.insert("value".to_owned(), json!(shown));
    }
    Ok(Answer::of(data))
}

pub fn add_sketch(
    session: &mut Session,
    work: &Work<'_>,
    asked: NewSketch,
) -> Result<Answer, ToolError> {
    let (feature, transaction) = match (asked.plane, asked.body, asked.face) {
        (Some(_), _, Some(_)) => return Err(SketchingError::BothPlaces.into()),
        (None, _, None) => return Err(SketchingError::NoPlace.into()),
        (None, None, Some(_)) => return Err(SketchingError::FaceWithoutBody.into()),
        (Some(name), _, None) => {
            let principal = match name {
                PlaneName::Xy => PrincipalPlane::Xy,
                PlaneName::Xz => PrincipalPlane::Xz,
                PlaneName::Yz => PrincipalPlane::Yz,
            };
            let document = session.document();
            let sketch_name = asked
                .name
                .unwrap_or_else(|| editing::next_sketch_name(document));
            let mut transaction = document.transaction(format!("Create {sketch_name}"));
            let feature = transaction.add_feature(
                sketch_name,
                FeatureKind::from(Sketch::new(principal.plane())),
            );
            (feature, transaction.finish())
        }
        (None, Some(body), Some(face)) => {
            session.evaluate(work)?;
            let document = session.document();
            let evaluation = session.evaluation();
            let body = references::body(document, evaluation, &body)?;
            let (key, _, _) = references::face(document, evaluation, body, &face)?;
            let (attachment, on) = sketch_placement::attachment_in(
                document,
                evaluation,
                FaceChoice { body, face: key },
                document.bar_index(),
            )
            .map_err(|reason| SketchingError::UnusableFace { reason })?;
            let sketch_name = asked
                .name
                .unwrap_or_else(|| editing::next_sketch_name(document));
            let mut transaction = document.transaction(format!("Create {sketch_name}"));
            let feature = transaction.add_feature(
                sketch_name,
                FeatureKind::Sketch(SketchFeature::on_face(Sketch::new(on), attachment)),
            );
            (feature, transaction.finish())
        }
    };
    let answer = apply(session, work, transaction, Map::new())?;
    let mut data = answer.data;
    if let Some(added) = session.document().feature(feature) {
        let mut sketch = Map::new();
        sketch.insert("id".to_owned(), json!(added.id().raw()));
        sketch.insert("name".to_owned(), json!(added.name));
        let shown = up_to_date_result(session.evaluation(), feature)
            .map(|result| result.geometry.plane())
            .or_else(|| added.kind.sketch().map(Sketch::plane));
        if let Some(shown) = shown {
            sketch.insert("plane".to_owned(), plane(&shown));
        }
        data.insert("sketch".to_owned(), Value::Object(sketch));
    }
    Ok(Answer::of(data))
}

fn coordinate(value: f64) -> Result<f64, SketchingError> {
    if value.is_finite() && value.abs() <= MAX_SIZE {
        Ok(value)
    } else {
        Err(SketchingError::Coordinate(value))
    }
}

fn point(at: [f64; 2]) -> Result<Point2, SketchingError> {
    let [x, y] = at;
    Ok(Point2::new(coordinate(x)?, coordinate(y)?))
}

fn ids(ids: &[EntityId]) -> Value {
    Value::Array(ids.iter().map(|id| json!(id.raw())).collect())
}

struct Pen<'t, 'd> {
    transaction: &'t mut TransactionBuilder<'d>,
    sketch: FeatureId,
    construction: bool,
}

impl Pen<'_, '_> {
    fn point(&mut self, at: Point2) -> EntityId {
        self.transaction
            .add_sketch_entity(self.sketch, Entity::Point(at))
    }

    fn line(&mut self, start: EntityId, end: EntityId) -> EntityId {
        self.transaction.add_sketch_entity_as(
            self.sketch,
            Entity::Line { start, end },
            self.construction,
        )
    }

    fn constrain(&mut self, constraint: Constraint) -> ConstraintId {
        self.transaction
            .add_sketch_constraint(self.sketch, constraint)
    }

    fn draw(&mut self, item: &Item) -> Result<Value, SketchingError> {
        Ok(match item {
            Item::Point { at } => {
                let id = self.point(point(*at)?);
                json!({ "kind": "point", "id": id.raw() })
            }
            Item::Line { from, to } => {
                let (from, to) = (point(*from)?, point(*to)?);
                if from == to {
                    return Err(SketchingError::ZeroLine);
                }
                let start = self.point(from);
                let end = self.point(to);
                let line = self.line(start, end);
                json!({ "kind": "line", "id": line.raw(), "start": start.raw(), "end": end.raw() })
            }
            Item::Polyline { points, closed } => {
                if points.len() < 2 {
                    return Err(SketchingError::ShortPolyline);
                }
                let placed: Vec<Point2> = points
                    .iter()
                    .map(|at| point(*at))
                    .collect::<Result<_, _>>()?;
                let corners: Vec<EntityId> = placed.iter().map(|at| self.point(*at)).collect();
                let mut lines = Vec::new();
                for pair in corners.windows(2) {
                    if let [start, end] = pair {
                        lines.push(self.line(*start, *end));
                    }
                }
                if *closed
                    && corners.len() > 2
                    && let (Some(first), Some(last)) = (corners.first(), corners.last())
                {
                    lines.push(self.line(*last, *first));
                }
                json!({ "kind": "polyline", "points": ids(&corners), "lines": ids(&lines) })
            }
            Item::Rectangle { corner, opposite } => {
                let (corner, opposite) = (point(*corner)?, point(*opposite)?);
                if corner.x == opposite.x || corner.y == opposite.y {
                    return Err(SketchingError::FlatRectangle);
                }
                let (low, high) = (corner.min(opposite), corner.max(opposite));
                let corners = [
                    self.point(low),
                    self.point(Point2::new(high.x, low.y)),
                    self.point(high),
                    self.point(Point2::new(low.x, high.y)),
                ];
                let [lower_left, lower_right, upper_right, upper_left] = corners;
                let sides = [
                    self.line(lower_left, lower_right),
                    self.line(lower_right, upper_right),
                    self.line(upper_right, upper_left),
                    self.line(upper_left, lower_left),
                ];
                let [bottom, right, top, left] = sides;
                for horizontal in [bottom, top] {
                    self.constrain(Constraint::Horizontal(horizontal));
                }
                for vertical in [right, left] {
                    self.constrain(Constraint::Vertical(vertical));
                }
                json!({
                    "kind": "rectangle",
                    "corners": ids(&corners),
                    "corner_names": ["lower left", "lower right", "upper right", "upper left"],
                    "sides": ids(&sides),
                    "side_names": ["bottom", "right", "top", "left"],
                })
            }
            Item::Circle { center, radius } => {
                if !radius.is_finite() || *radius <= 0.0 || *radius > MAX_SIZE {
                    return Err(SketchingError::Radius(*radius));
                }
                let centre = self.point(point(*center)?);
                let circle = self.transaction.add_sketch_entity_as(
                    self.sketch,
                    Entity::Circle {
                        center: centre,
                        radius: *radius,
                    },
                    self.construction,
                );
                json!({ "kind": "circle", "id": circle.raw(), "center": centre.raw() })
            }
        })
    }
}

pub fn draw(session: &mut Session, work: &Work<'_>, asked: Drawing) -> Result<Answer, ToolError> {
    if asked.items.is_empty() {
        return Err(SketchingError::Nothing.into());
    }
    if asked.items.len() > MOST_ITEMS {
        return Err(SketchingError::TooMany.into());
    }
    session.evaluate(work)?;
    let document = session.document();
    let (feature, _) = references::sketch(document, &asked.sketch)?;
    let sketch = feature.id();
    let mut transaction = document.transaction(format!("Draw in {}", feature.name));
    settled(&mut transaction, session.evaluation(), sketch);
    let mut pen = Pen {
        transaction: &mut transaction,
        sketch,
        construction: asked.construction,
    };
    let drawn: Vec<Value> = asked
        .items
        .iter()
        .map(|item| pen.draw(item))
        .collect::<Result<_, _>>()?;
    let transaction = transaction.finish();
    let mut details = Map::new();
    details.insert("drawn".to_owned(), Value::Array(drawn));
    apply(session, work, transaction, details)
}

struct Resolver<'s> {
    name: &'s str,
    sketch: &'s Sketch,
}

impl Resolver<'_> {
    fn entity(&self, named: &EntityName) -> Result<EntityId, ToolError> {
        Ok(references::entity(self.name, self.sketch, named)?)
    }

    fn pair(&self, a: &EntityName, b: &EntityName) -> Result<(EntityId, EntityId), ToolError> {
        Ok((self.entity(a)?, self.entity(b)?))
    }
}

fn constraint_of(
    session: &Session,
    resolver: &Resolver<'_>,
    item: &ConstraintItem,
) -> Result<Constraint, ToolError> {
    let length = |text: &str| value(session, "value", text, Dimension::LENGTH);
    let angle = |text: &str| value(session, "value", text, Dimension::ANGLE);
    Ok(match item {
        ConstraintItem::Coincident { a, b } => {
            let (a, b) = resolver.pair(a, b)?;
            Constraint::Coincident(a, b)
        }
        ConstraintItem::Horizontal { line } => Constraint::Horizontal(resolver.entity(line)?),
        ConstraintItem::Vertical { line } => Constraint::Vertical(resolver.entity(line)?),
        ConstraintItem::Parallel { a, b } => {
            let (a, b) = resolver.pair(a, b)?;
            Constraint::Parallel(a, b)
        }
        ConstraintItem::Perpendicular { a, b } => {
            let (a, b) = resolver.pair(a, b)?;
            Constraint::Perpendicular(a, b)
        }
        ConstraintItem::Equal { a, b } => {
            let (a, b) = resolver.pair(a, b)?;
            Constraint::Equal(a, b)
        }
        ConstraintItem::Tangent { a, b } => {
            let (a, b) = resolver.pair(a, b)?;
            Constraint::Tangent(a, b)
        }
        ConstraintItem::Concentric { a, b } => {
            let (a, b) = resolver.pair(a, b)?;
            Constraint::Concentric(a, b)
        }
        ConstraintItem::Collinear { a, b } => {
            let (a, b) = resolver.pair(a, b)?;
            Constraint::Collinear(a, b)
        }
        ConstraintItem::Midpoint { point, curve } => Constraint::Midpoint {
            point: resolver.entity(point)?,
            curve: resolver.entity(curve)?,
        },
        ConstraintItem::Symmetric { a, b, about } => {
            let (first, second) = resolver.pair(a, b)?;
            Constraint::Symmetric {
                first,
                second,
                about: resolver.entity(about)?,
            }
        }
        ConstraintItem::Fix { point } => {
            let id = resolver.entity(point)?;
            let at = resolver
                .sketch
                .point(id)
                .filter(|_| matches!(resolver.sketch.entity(id), Some(Entity::Point(_))))
                .ok_or_else(|| SketchingError::NotAPoint {
                    sketch: resolver.name.to_owned(),
                    entity: point.clone(),
                })?;
            Constraint::Fix { point: id, at }
        }
        ConstraintItem::Distance { from, to, value } => {
            let (from, to) = resolver.pair(from, to)?;
            Constraint::Distance {
                from,
                to,
                value: length(value)?,
            }
        }
        ConstraintItem::HorizontalDistance { from, to, value } => {
            let (from, to) = resolver.pair(from, to)?;
            Constraint::HorizontalDistance {
                from,
                to,
                value: length(value)?,
            }
        }
        ConstraintItem::VerticalDistance { from, to, value } => {
            let (from, to) = resolver.pair(from, to)?;
            Constraint::VerticalDistance {
                from,
                to,
                value: length(value)?,
            }
        }
        ConstraintItem::Radius { curve, value } => Constraint::Radius {
            entity: resolver.entity(curve)?,
            value: length(value)?,
        },
        ConstraintItem::Diameter { curve, value } => Constraint::Diameter {
            entity: resolver.entity(curve)?,
            value: length(value)?,
        },
        ConstraintItem::Angle { from, to, value } => {
            let (from, to) = resolver.pair(from, to)?;
            Constraint::Angle {
                from,
                to,
                reversed: false,
                value: angle(value)?,
            }
        }
    })
}

pub fn constrain(
    session: &mut Session,
    work: &Work<'_>,
    asked: Constraining,
) -> Result<Answer, ToolError> {
    if asked.constraints.is_empty() {
        return Err(SketchingError::Nothing.into());
    }
    if asked.constraints.len() > MOST_ITEMS {
        return Err(SketchingError::TooMany.into());
    }
    session.evaluate(work)?;
    let document = session.document();
    let (feature, definition) = references::sketch(document, &asked.sketch)?;
    let sketch = feature.id();
    let shown = up_to_date_result(session.evaluation(), sketch)
        .map_or(definition, |result| &result.geometry);
    let resolver = Resolver {
        name: &feature.name,
        sketch: shown,
    };
    let constraints: Vec<Constraint> = asked
        .constraints
        .iter()
        .map(|item| constraint_of(session, &resolver, item))
        .collect::<Result<_, _>>()?;
    let mut transaction = document.transaction(format!("Constrain {}", feature.name));
    settled(&mut transaction, session.evaluation(), sketch);
    let added: Vec<ConstraintId> = constraints
        .into_iter()
        .map(|constraint| transaction.add_sketch_constraint(sketch, constraint))
        .collect();
    let transaction = transaction.finish();
    let trial = ConstraintTrial {
        feature: sketch,
        transaction,
        added: added.clone(),
    };
    let started = Instant::now();
    let given_up = || work.is_cancelled() || started.elapsed() >= PATIENCE;
    if let Some(reason) = constraint_trial::conflict(document, &trial, &given_up) {
        return Err(SketchingError::Conflicting { reason }.into());
    }
    let mut details = Map::new();
    details.insert(
        "constraints".to_owned(),
        Value::Array(added.iter().map(|id| json!(id.raw())).collect()),
    );
    let answer = apply(session, work, trial.transaction, details)?;
    let mut data = answer.data;
    let summary = crate::sketch_status::SketchSummary::of(session.evaluation(), sketch);
    data.insert("sketch_status".to_owned(), json!(summary.status_text()));
    Ok(Answer::of(data))
}

fn operation_words(document: &Document, operation: BodyOperation) -> String {
    match operation {
        BodyOperation::NewBody => "makes a new body".to_owned(),
        BodyOperation::Add(body) => format!("adds to {}", body_label(document, body)),
        BodyOperation::Remove(body) => format!("removes from {}", body_label(document, body)),
        BodyOperation::Intersect(body) => {
            format!("keeps what it shares with {}", body_label(document, body))
        }
    }
}

pub fn extrude(
    session: &mut Session,
    work: &Work<'_>,
    asked: Extrusion,
) -> Result<Answer, ToolError> {
    let cuts = matches!(
        asked.operation,
        Some(OperationName::Remove | OperationName::Intersect)
    );
    let end = match (&asked.distance, asked.through_all) {
        (Some(_), true) => return Err(ModellingError::DistanceAndThroughAll.into()),
        (None, false) => return Err(ModellingError::NoDistance.into()),
        (None, true) if asked.symmetric => {
            return Err(ModellingError::SymmetricThroughAll.into());
        }
        (None, true) if asked.operation.is_some() && !cuts => {
            return Err(ModellingError::ThroughAllAdds.into());
        }
        (None, true) => ExtrudeEnd::ThroughAll,
        (Some(distance), false) => {
            ExtrudeEnd::Distance(value(session, "distance", distance, Dimension::LENGTH)?)
        }
    };
    session.evaluate(work)?;
    let values = session.parameters();
    let document = session.document();
    let evaluation = session.evaluation();
    let (feature, _) = references::sketch(document, &asked.sketch)?;
    let sketch = feature.id();
    let chosen = asked
        .body
        .as_ref()
        .map(|body| references::body(document, evaluation, body))
        .transpose()?;
    let standing_on = solid_tools::face_body(document, sketch);
    let extent = match (asked.symmetric, end) {
        (true, ExtrudeEnd::Distance(distance)) => ExtrudeExtent::Symmetric { distance },
        (_, end) => ExtrudeExtent::OneSide {
            end,
            reversed: false,
        },
    };
    let template = Extrude {
        sketch,
        regions: RegionChoice::All,
        extent,
        operation: BodyOperation::NewBody,
        start: None,
        other_bodies: Vec::new(),
        taper: None,
        wall: None,
        direction: None,
    };
    let (solid, told) = match asked.operation {
        None => {
            let detected = extrusion_detection::extrusion(
                document,
                Reach {
                    evaluation,
                    parameters: &values,
                },
                &template,
                standing_on,
                chosen,
            );
            let solid = Extrude {
                extent: detected.extent,
                operation: detected.operation,
                ..template
            };
            (solid, detected.told)
        }
        Some(name) => {
            let target = chosen
                .or(standing_on)
                .or_else(|| document.bodies_standing().last().copied());
            let operation = match (name, target) {
                (OperationName::NewBody, _) => BodyOperation::NewBody,
                (OperationName::Add, Some(body)) => BodyOperation::Add(body),
                (OperationName::Remove, Some(body)) => BodyOperation::Remove(body),
                (OperationName::Intersect, Some(body)) => BodyOperation::Intersect(body),
                (OperationName::Add, None) => {
                    return Err(ModellingError::NoTarget { operation: "add" }.into());
                }
                (OperationName::Remove, None) => {
                    return Err(ModellingError::NoTarget {
                        operation: "remove",
                    }
                    .into());
                }
                (OperationName::Intersect, None) => {
                    return Err(ModellingError::NoTarget {
                        operation: "intersect",
                    }
                    .into());
                }
            };
            let into_its_face = cuts && target.is_some() && target == standing_on;
            let reversed = asked.reversed.unwrap_or(into_its_face);
            let mut solid = Extrude {
                operation,
                ..template
            };
            if let ExtrudeExtent::OneSide {
                reversed: turned, ..
            } = &mut solid.extent
            {
                *turned = reversed;
            }
            (solid, None)
        }
    };
    let name = asked
        .name
        .unwrap_or_else(|| editing::next_feature_name(document, Sweep::Extrude.label()));
    let operation = solid.operation;
    let mut transaction = document.transaction(format!("Create {name}"));
    let added = transaction.add_feature(name, FeatureKind::Solid(SolidFeature::Extrude(solid)));
    if visibility::is_shown(document, sketch) {
        transaction.edit(Edit::SetFeatureHidden {
            id: sketch,
            hidden: true,
        });
    }
    let mut details = Map::new();
    details.insert(
        "operation".to_owned(),
        json!(operation_words(document, operation)),
    );
    if let Some(told) = told {
        details.insert("note".to_owned(), json!(told));
    }
    let transaction = transaction.finish();
    let answer = apply(session, work, transaction, details)?;
    Ok(with_feature(session, answer, added))
}

fn with_feature(session: &Session, answer: Answer, feature: FeatureId) -> Answer {
    let mut data = answer.data;
    if let Some(added) = session.document().feature(feature) {
        data.insert("feature".to_owned(), feature_brief(added));
    }
    Answer::of(data)
}

pub fn drill(session: &mut Session, work: &Work<'_>, asked: Drilling) -> Result<Answer, ToolError> {
    let diameter = value(session, "diameter", &asked.diameter, Dimension::LENGTH)?;
    let through = THROUGH_ALL
        .iter()
        .any(|words| asked.depth.trim().eq_ignore_ascii_case(words));
    let depth = if through {
        HoleDepth::ThroughAll
    } else {
        HoleDepth::Blind(value(session, "depth", &asked.depth, Dimension::LENGTH)?)
    };
    session.evaluate(work)?;
    let document = session.document();
    let evaluation = session.evaluation();
    let (feature, definition) = references::sketch(document, &asked.sketch)?;
    let sketch = feature.id();
    if hole_centres(definition).is_empty() {
        return Err(ModellingError::NothingToDrill {
            sketch: feature.name.clone(),
        }
        .into());
    }
    let body = match &asked.body {
        Some(body) => references::body(document, evaluation, body)?,
        None => solid_tools::face_body(document, sketch)
            .or_else(|| document.bodies_standing().last().copied())
            .ok_or(ModellingError::NoBodyToDrill)?,
    };
    let mut kind = hole_tools::new_hole(sketch, body, &Starts::defaults(LengthUnit::Millimetre));
    if let FeatureKind::Hole(hole) = &mut kind {
        hole.diameter = diameter;
        hole.depth = depth;
        hole.reversed = asked.reversed;
    }
    let name = asked
        .name
        .unwrap_or_else(|| editing::next_feature_name(document, hole_tools::TITLE));
    let mut transaction = document.transaction(format!("Create {name}"));
    let added = transaction.add_feature(name, kind);
    if visibility::is_shown(document, sketch) {
        transaction.edit(Edit::SetFeatureHidden {
            id: sketch,
            hidden: true,
        });
    }
    let mut details = Map::new();
    details.insert("holes".to_owned(), json!(hole_centres(definition).len()));
    details.insert("body".to_owned(), json!(body_label(document, body)));
    let transaction = transaction.finish();
    let answer = apply(session, work, transaction, details)?;
    Ok(with_feature(session, answer, added))
}

pub fn undo(session: &mut Session, work: &Work<'_>) -> Result<Answer, ToolError> {
    let before = failing_ids(session);
    let performed = session.perform(Change::Undo)?;
    let mut details = Map::new();
    details.insert("undone".to_owned(), json!(performed.label));
    report(session, work, &performed, before.as_ref(), details)
}

pub fn redo(session: &mut Session, work: &Work<'_>) -> Result<Answer, ToolError> {
    let before = failing_ids(session);
    let performed = session.perform(Change::Redo)?;
    let mut details = Map::new();
    details.insert("redone".to_owned(), json!(performed.label));
    report(session, work, &performed, before.as_ref(), details)
}
