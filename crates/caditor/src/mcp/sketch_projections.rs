use caditor_document::{Document, Evaluation, Feature, FeatureKind, Outline, ProjectionSource};
use caditor_geometry::Point3;
use serde::Deserialize;

use crate::{
    bodies, datum_tools,
    display::Displayed,
    feature_tree::count,
    mcp::{
        places::{AxisName, PlaceError, principal_plane},
        references::{self, EntityName, Named, body_label},
        sketch_edits::{Planned, SketchEditError, Target},
        tools::ToolError,
        writing::{self, MOST_ITEMS, PlaneName, SketchingError},
    },
    projecting::{self, Surroundings},
    selection::{Axis, Pickable},
    sketch_status::up_to_date_result,
};

const CORNER_TOLERANCE: f64 = 1e-6;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Projecting {
    pub sketch: Named,
    pub body: Option<Named>,
    #[serde(default)]
    pub edges: Vec<String>,
    #[serde(default)]
    pub faces: Vec<String>,
    #[serde(default)]
    pub corners: Vec<[f64; 3]>,
    pub source_sketch: Option<Named>,
    #[serde(default)]
    pub source_curves: Vec<EntityName>,
    #[serde(default)]
    pub axes: Vec<AxisName>,
    #[serde(default)]
    pub datums: Vec<Named>,
    #[serde(default)]
    pub origin: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Intersecting {
    pub sketch: Named,
    pub body: Option<Named>,
    pub face: Option<String>,
    pub datum: Option<Named>,
    pub plane: Option<PlaneName>,
}

struct Seen<'s> {
    document: &'s Document,
    evaluation: &'s Evaluation,
}

impl Surroundings for Seen<'_> {
    fn document(&self) -> &Document {
        self.document
    }

    fn evaluation(&self) -> &Evaluation {
        self.evaluation
    }

    fn shown_sketch<'a>(&'a self, feature: &'a Feature) -> Option<Displayed<'a>> {
        up_to_date_result(self.evaluation, feature.id())
            .map(|result| Displayed::Borrowed(&result.geometry))
            .or_else(|| feature.kind.sketch().map(Displayed::Borrowed))
    }
}

type Wanted = Vec<(ProjectionSource, Outline)>;

fn seen_around<'s>(target: &Target<'s>) -> Seen<'s> {
    Seen {
        document: target.document,
        evaluation: target.evaluation,
    }
}

fn projections(
    surroundings: &Seen<'_>,
    target: &Target<'_>,
    pickable: Pickable,
) -> Result<Wanted, ToolError> {
    projecting::projections_of(
        surroundings,
        target.feature,
        &target.shown.plane(),
        pickable,
    )
    .map_err(|error| SketchEditError::Projection(error).into())
}

fn body_things(
    target: &Target<'_>,
    surroundings: &Seen<'_>,
    asked: &Projecting,
    wanted: &mut Wanted,
) -> Result<Option<String>, ToolError> {
    let needs_body =
        !(asked.edges.is_empty() && asked.faces.is_empty() && asked.corners.is_empty());
    let Some(named) = &asked.body else {
        return if needs_body {
            Err(SketchEditError::BodyNeeded.into())
        } else {
            Ok(None)
        };
    };
    let document = target.document;
    let body = references::body(document, target.evaluation, named)?;
    let body_name = body_label(document, body);
    let seen = target
        .evaluation
        .body_result_seen_by(target.feature, body)
        .ok_or_else(|| SketchEditError::LaterBody {
            body: body_name.clone(),
            sketch: target.name.to_owned(),
        })?;
    let not_seen = |what: &'static str, key: &str| SketchEditError::NotSeen {
        body: body_name.clone(),
        what,
        key: key.to_owned(),
        sketch: target.name.to_owned(),
    };
    let plane = target.shown.plane();
    for key in &asked.edges {
        let name = super::places::edge_key(key)?;
        let edge = bodies::find_edge(seen, name).ok_or_else(|| not_seen("edge", key))?;
        let projection =
            projecting::edge_projection(seen, body, edge, &plane).ok_or_else(|| {
                SketchEditError::NotProjectable {
                    body: body_name.clone(),
                    key: key.clone(),
                    sketch: target.name.to_owned(),
                }
            })?;
        wanted.push(projection);
    }
    for key in &asked.faces {
        let face_key = references::face_key(key)?;
        let face = bodies::find_face(seen, face_key).ok_or_else(|| not_seen("face", key))?;
        wanted.extend(projecting::face_projections(seen, body, &[face], &plane));
    }
    for corner in &asked.corners {
        let [x, y, z] = *corner;
        let at = Point3::new(x, y, z);
        let reach = CORNER_TOLERANCE * at.length().max(1.0);
        let keys = bodies::vertex_keys(seen);
        let vertex = seen
            .solid
            .vertices()
            .filter_map(|(id, vertex)| {
                let apart = vertex.point().distance(at);
                let key = keys.iter().find(|(found, _)| *found == id)?.1;
                (apart <= reach).then_some((apart, key))
            })
            .min_by(|one, other| one.0.total_cmp(&other.0))
            .map(|(_, key)| key)
            .ok_or_else(|| PlaceError::NoCorner {
                body: body_name.clone(),
                x,
                y,
                z,
            })?;
        wanted.extend(projections(
            surroundings,
            target,
            Pickable::Vertex { body, vertex },
        )?);
    }
    Ok((!asked.edges.is_empty() || !asked.faces.is_empty()).then_some(body_name))
}

fn other_things(
    target: &Target<'_>,
    surroundings: &Seen<'_>,
    asked: &Projecting,
    wanted: &mut Wanted,
) -> Result<(), ToolError> {
    let document = target.document;
    if let Some(named) = &asked.source_sketch {
        let (source, definition) = references::sketch(document, named)?;
        if asked.source_curves.is_empty() {
            let (_, candidates) =
                projecting::sketch_candidates(surroundings, target.feature, source.id())
                    .map_err(SketchEditError::Projection)?;
            wanted.extend(candidates);
        } else {
            let shown = surroundings
                .shown_sketch(source)
                .unwrap_or(Displayed::Borrowed(definition));
            for curve in &asked.source_curves {
                let entity = references::entity(&source.name, &shown, curve)?;
                wanted.extend(projections(
                    surroundings,
                    target,
                    Pickable::SketchEntity {
                        feature: source.id(),
                        entity,
                    },
                )?);
            }
        }
    } else if !asked.source_curves.is_empty() {
        return Err(SketchEditError::CurvesWithoutSketch.into());
    }
    for axis in &asked.axes {
        wanted.extend(projections(
            surroundings,
            target,
            Pickable::Axis(Axis::of(axis.principal())),
        )?);
    }
    for named in &asked.datums {
        let datum = references::feature(document, named)?;
        if !datum_tools::is_axis_or_point(document, datum.id()) {
            return Err(PlaceError::NotADatum {
                name: datum.name.clone(),
                kind: "axis or point",
            }
            .into());
        }
        wanted.extend(projections(
            surroundings,
            target,
            Pickable::Datum(datum.id()),
        )?);
    }
    if asked.origin {
        wanted.extend(projections(surroundings, target, Pickable::Origin)?);
    }
    Ok(())
}

pub(super) fn project(target: &Target<'_>, asked: &Projecting) -> Result<Planned, ToolError> {
    let named = asked.edges.len()
        + asked.faces.len()
        + asked.corners.len()
        + asked.source_curves.len()
        + asked.axes.len()
        + asked.datums.len()
        + usize::from(asked.origin)
        + usize::from(asked.source_sketch.is_some());
    if named == 0 {
        return Err(SketchEditError::NothingToProject.into());
    }
    if named > MOST_ITEMS {
        return Err(SketchingError::TooMany.into());
    }
    let document = target.document;
    let surroundings = seen_around(target);
    let mut wanted = Wanted::new();
    let body_name = body_things(target, &surroundings, asked, &mut wanted)?;
    other_things(target, &surroundings, asked, &mut wanted)?;
    let definition = match document.feature(target.feature).map(|found| &found.kind) {
        Some(FeatureKind::Sketch(definition)) => Some(definition),
        _ => None,
    };
    let mut fresh = Wanted::new();
    for (source, outline) in wanted {
        let projected = definition.is_some_and(|definition| {
            projecting::existing_projection(definition, &source).is_some()
        });
        let repeated = fresh.iter().any(|(taken, _)| taken.is_same_source(&source));
        if !projected && !repeated {
            fresh.push((source, outline));
        }
    }
    if fresh.is_empty() {
        return Err(SketchEditError::AlreadyProjected {
            sketch: target.name.to_owned(),
        }
        .into());
    }
    let label = match (&body_name, fresh.len()) {
        (Some(body_name), total) => {
            format!("Project {} of {body_name}", count(total, "edge", "edges"))
        }
        (None, total) => format!(
            "Project {}",
            count(total, "curve or point", "curves and points")
        ),
    };
    let mut transaction = document.transaction(label);
    let projected: Vec<_> = fresh
        .iter()
        .map(|(source, outline)| {
            transaction.add_projection(target.feature, source.clone(), outline)
        })
        .collect();
    let mut planned = Planned::of(transaction.finish());
    planned
        .details
        .insert("projected".to_owned(), writing::ids(&projected));
    Ok(planned)
}

pub(super) fn intersect(target: &Target<'_>, asked: &Intersecting) -> Result<Planned, ToolError> {
    let surroundings = seen_around(target);
    let document = target.document;
    let (pickable, whole) = match (&asked.body, &asked.datum, asked.plane) {
        (Some(named), None, None) => {
            let body = references::body(document, target.evaluation, named)?;
            let seen = target
                .evaluation
                .body_result_seen_by(target.feature, body)
                .ok_or_else(|| SketchEditError::LaterBody {
                    body: body_label(document, body),
                    sketch: target.name.to_owned(),
                })?;
            match &asked.face {
                Some(key) => {
                    let face = references::face_key(key)?;
                    bodies::find_face(seen, face).ok_or_else(|| SketchEditError::NotSeen {
                        body: body_label(document, body),
                        what: "face",
                        key: key.clone(),
                        sketch: target.name.to_owned(),
                    })?;
                    (Pickable::Face { body, face }, false)
                }
                None => {
                    let (_, face) = bodies::face_keys(&seen.solid)
                        .into_iter()
                        .next()
                        .ok_or_else(|| SketchEditError::NotSeen {
                            body: body_label(document, body),
                            what: "face",
                            key: String::new(),
                            sketch: target.name.to_owned(),
                        })?;
                    (Pickable::Face { body, face }, true)
                }
            }
        }
        (None, Some(named), None) => {
            if asked.face.is_some() {
                return Err(SketchEditError::IntersectWays.into());
            }
            let datum = references::feature(document, named)?;
            (Pickable::Datum(datum.id()), false)
        }
        (None, None, Some(plane)) => (Pickable::Plane(principal_plane(plane)), false),
        _ => return Err(SketchEditError::IntersectWays.into()),
    };
    if asked.face.is_some() && asked.body.is_none() {
        return Err(SketchEditError::IntersectWays.into());
    }
    let transaction = projecting::intersect(&surroundings, target.feature, pickable, whole)
        .map_err(SketchEditError::Projection)?;
    Ok(Planned::of(transaction))
}
