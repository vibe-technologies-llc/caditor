use std::f64::consts::TAU;

use caditor_geometry::Point3;
use caditor_kernel::{
    Accuracy, AngleKind, EdgeForm, Element, MeasureError, angle, distance, edge_measure,
    interruptible,
};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::{
    bodies,
    mcp::{
        edits::viewed_before,
        places::{self, OnEdge, OnFace, edge_key_text},
        reading::{OfBody, point3, rounded, vector3},
        references::{self, ReferenceError, body_label},
        session::{Cancelled, Session, Work},
        tools::ToolError,
    },
};

pub const MOST_EDGES: usize = 500;
const FULL_CIRCLE_TOLERANCE: f64 = 1e-9;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum Spot {
    Point([f64; 3]),
    Face(OnFace),
    Edge(OnEdge),
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Between {
    pub from: Spot,
    pub to: Spot,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum MeasuringError {
    #[error("{0} is not a point of finite coordinates")]
    UnusablePoint(String),
    #[error("the distance could not be worked out: {0}")]
    Distance(MeasureError),
}

pub fn edges(
    session: &mut Session,
    work: &Work<'_>,
    of: &OfBody,
) -> Result<Map<String, Value>, ToolError> {
    session.evaluate(work)?;
    viewed_before(session, of.before.as_ref(), |session| {
        listed_edges(session, work, of)
    })
}

fn listed_edges(
    session: &Session,
    work: &Work<'_>,
    of: &OfBody,
) -> Result<Map<String, Value>, ToolError> {
    let document = session.document();
    let evaluation = session.evaluation();
    let body = references::body(document, evaluation, &of.body)?;
    let shown =
        bodies::shown(evaluation, body).ok_or_else(|| ReferenceError::NoBody(of.body.clone()))?;
    let total = shown.solid.edges().count();
    let mut listed = Vec::with_capacity(total.min(MOST_EDGES));
    for (id, edge) in shown.solid.edges().take(MOST_EDGES) {
        if work.is_cancelled() {
            return Err(Cancelled.into());
        }
        let mut entry = Map::new();
        entry.insert("edge".to_owned(), json!(edge_key_text(edge.name())));
        entry.insert(
            "description".to_owned(),
            json!(bodies::describe_edge_id(document, shown, id)),
        );
        if bodies::is_seam(&shown.solid, id) {
            entry.insert("seam".to_owned(), json!(true));
        }
        match interruptible(work.interrupt(), || edge_measure(&shown.solid, id)) {
            Ok(measured) => {
                describe_form(measured.form, &mut entry);
                entry.insert("length".to_owned(), rounded(measured.length));
                if measured.length_accuracy == Accuracy::Approximate {
                    entry.insert("length_exact".to_owned(), json!(false));
                }
            }
            Err(MeasureError::Cancelled(_)) => return Err(Cancelled.into()),
            Err(_) => {
                entry.insert("kind".to_owned(), json!("curve"));
            }
        }
        listed.push(Value::Object(entry));
    }
    let mut answer = Map::new();
    answer.insert("body".to_owned(), json!(body_label(document, body)));
    answer.insert("edges".to_owned(), Value::Array(listed));
    if total > MOST_EDGES {
        answer.insert(
            "left_out".to_owned(),
            json!(format!(
                "{} more edges are not listed",
                total.saturating_sub(MOST_EDGES)
            )),
        );
    }
    Ok(answer)
}

fn describe_form(form: EdgeForm, entry: &mut Map<String, Value>) {
    match form {
        EdgeForm::Line { start, end } => {
            entry.insert("kind".to_owned(), json!("line"));
            entry.insert("start".to_owned(), point3(start));
            entry.insert("end".to_owned(), point3(end));
        }
        EdgeForm::Circle {
            center,
            radius,
            normal,
            sweep,
        } => {
            let whole = (sweep - TAU).abs() <= FULL_CIRCLE_TOLERANCE;
            entry.insert(
                "kind".to_owned(),
                json!(if whole { "circle" } else { "arc" }),
            );
            entry.insert("center".to_owned(), point3(center));
            entry.insert("radius".to_owned(), rounded(radius));
            entry.insert("normal".to_owned(), vector3(normal));
            if !whole {
                entry.insert("sweep".to_owned(), rounded(sweep.to_degrees()));
            }
        }
        EdgeForm::Ellipse {
            center,
            major_radius,
            minor_radius,
        } => {
            entry.insert("kind".to_owned(), json!("ellipse"));
            entry.insert("center".to_owned(), point3(center));
            entry.insert("major_radius".to_owned(), rounded(major_radius));
            entry.insert("minor_radius".to_owned(), rounded(minor_radius));
        }
        EdgeForm::Curve => {
            entry.insert("kind".to_owned(), json!("curve"));
        }
    }
}

pub fn measure(
    session: &mut Session,
    work: &Work<'_>,
    between: &Between,
) -> Result<Map<String, Value>, ToolError> {
    session.evaluate(work)?;
    let document = session.document();
    let evaluation = session.evaluation();
    let element = |spot: &Spot| -> Result<(Element<'_>, String), ToolError> {
        match spot {
            Spot::Point(at) => {
                let [x, y, z] = *at;
                if !(x.is_finite() && y.is_finite() && z.is_finite()) {
                    return Err(MeasuringError::UnusablePoint(format!("{at:?}")).into());
                }
                Ok((
                    Element::Point(Point3::new(x, y, z)),
                    format!("the point ({x}, {y}, {z})"),
                ))
            }
            Spot::Face(on) => {
                let body = references::body(document, evaluation, &on.body)?;
                let (_, face, shown) = references::face(document, evaluation, body, &on.face)?;
                Ok((
                    Element::Face {
                        solid: &shown.solid,
                        face,
                    },
                    bodies::describe_face_id(document, shown, face),
                ))
            }
            Spot::Edge(on) => {
                let body = references::body(document, evaluation, &on.body)?;
                let (_, edge, shown) = places::edge(document, evaluation, body, &on.edge)?;
                Ok((
                    Element::Edge {
                        solid: &shown.solid,
                        edge,
                    },
                    bodies::describe_edge_id(document, shown, edge),
                ))
            }
        }
    };
    let (from, from_words) = element(&between.from)?;
    let (to, to_words) = element(&between.to)?;
    let separation = match interruptible(work.interrupt(), || distance(from, to)) {
        Ok(separation) => separation,
        Err(MeasureError::Cancelled(_)) => return Err(Cancelled.into()),
        Err(error) => return Err(MeasuringError::Distance(error).into()),
    };
    let mut answer = Map::new();
    answer.insert("from".to_owned(), json!(from_words));
    answer.insert("to".to_owned(), json!(to_words));
    answer.insert("distance".to_owned(), rounded(separation.distance));
    answer.insert("closest_from".to_owned(), point3(separation.from));
    answer.insert("closest_to".to_owned(), point3(separation.to));
    answer.insert("offset".to_owned(), vector3(separation.offset()));
    answer.insert(
        "exact".to_owned(),
        json!(separation.accuracy == Accuracy::Exact),
    );
    match interruptible(work.interrupt(), || angle(from, to)) {
        Ok(Some(found)) => {
            answer.insert("angle".to_owned(), rounded(found.radians.to_degrees()));
            answer.insert("angle_between".to_owned(), json!(angle_words(found.kind)));
        }
        Ok(None) => {}
        Err(MeasureError::Cancelled(_)) => return Err(Cancelled.into()),
        Err(_) => {}
    }
    Ok(answer)
}

fn angle_words(kind: AngleKind) -> &'static str {
    match kind {
        AngleKind::Corner => "the edges at their shared corner",
        AngleKind::Lines => "the lines",
        AngleKind::Planes => "the planes",
        AngleKind::LineAndPlane => "the line and the plane",
    }
}
