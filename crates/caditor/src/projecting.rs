use std::collections::BTreeSet;

use caditor_document::{
    Document, FeatureId, FeatureKind, Outline, ProjectionSource, SketchFeature, SolidResult,
    Transaction, edge_outline, sketch_outline, vertex_outline,
};
use caditor_geometry::Plane;
use caditor_kernel::{EdgeId, EdgeReference, FaceId};

use crate::{bodies, body_selection::face_boundary, model::Model, selection::Pickable};

pub const ALREADY_PROJECTED: &str = "That is already projected into this sketch";
const NOTHING_TO_PROJECT: &str =
    "Every edge of that face is seen end-on from this sketch or already projected into it";

pub fn projectable(pickable: Pickable, sketch: FeatureId) -> bool {
    match pickable {
        Pickable::Edge { .. } | Pickable::Vertex { .. } | Pickable::Face { .. } => true,
        Pickable::SketchEntity { feature, entity } => feature != sketch && !entity.is_reference(),
        _ => false,
    }
}

pub fn describe(model: &Model, sketch: FeatureId, pickable: Pickable) -> Option<String> {
    let document = model.document();
    let what = match pickable {
        Pickable::Edge { body, edge } => {
            let result = seen(model, sketch, body)?;
            bodies::describe_edge(document, result, edge)
        }
        Pickable::Vertex { body, vertex } => {
            let result = seen(model, sketch, body)?;
            bodies::describe_vertex(document, result, vertex)
        }
        Pickable::Face { body, face } => {
            let result = seen(model, sketch, body)?;
            format!(
                "the edges of {}",
                bodies::describe_face(document, result, face)
            )
        }
        Pickable::SketchEntity { feature, entity } => {
            let owner = document.feature(feature)?;
            let source = owner.kind.sketch()?;
            format!("{} of {}", source.entity_label(entity), owner.name)
        }
        _ => return None,
    };
    Some(format!("Project {what}"))
}

pub fn project(
    model: &Model,
    sketch: FeatureId,
    pickable: Pickable,
) -> Result<Transaction, String> {
    let document = model.document();
    let edited = document
        .feature(sketch)
        .ok_or_else(|| "The sketch being edited no longer exists".to_owned())?;
    let FeatureKind::Sketch(definition) = &edited.kind else {
        return Err("Only a sketch can take projected geometry".to_owned());
    };
    let plane = model
        .displayed_sketch(edited)
        .map_or_else(|| definition.sketch.plane(), |displayed| displayed.plane());
    let projections = match pickable {
        Pickable::Edge { body, edge } => {
            let result = seen_or_refuse(model, sketch, body)?;
            let id =
                bodies::find_edge(result, edge).ok_or_else(|| later_geometry(document, sketch))?;
            vec![
                edge_projection(result, body, id, &plane)
                    .ok_or_else(|| "That edge cannot be projected into this sketch".to_owned())?,
            ]
        }
        Pickable::Vertex { body, vertex } => {
            let result = seen_or_refuse(model, sketch, body)?;
            let id = bodies::find_vertex(result, vertex)
                .ok_or_else(|| later_geometry(document, sketch))?;
            if result.names().vertices_named(vertex.name).len() > 1 {
                return Err(
                    "That corner cannot be told apart from another between the same faces, so \
                     the sketch could not follow it"
                        .to_owned(),
                );
            }
            let outline = vertex_outline(&result.solid, id, &plane)
                .ok_or_else(|| "That corner cannot be projected into this sketch".to_owned())?;
            vec![(
                ProjectionSource::Vertex {
                    body,
                    vertex: vertex.name,
                },
                outline,
            )]
        }
        Pickable::Face { body, face } => {
            let result = seen_or_refuse(model, sketch, body)?;
            let id =
                bodies::find_face(result, face).ok_or_else(|| later_geometry(document, sketch))?;
            face_projections(result, body, id, &plane)
        }
        Pickable::SketchEntity { feature, entity } => {
            let position = |id: FeatureId| document.features().position(|other| other.id() == id);
            if position(feature) >= position(sketch) {
                return Err(format!(
                    "{} comes after {} in the tree, so its geometry cannot be projected into it",
                    feature_name(document, feature),
                    edited.name
                ));
            }
            let owner = document
                .feature(feature)
                .ok_or_else(|| "That sketch no longer exists".to_owned())?;
            let source = model
                .displayed_sketch(owner)
                .ok_or_else(|| format!("{} has not been computed yet", owner.name))?;
            let outline = sketch_outline(&source, entity, &plane)
                .ok_or_else(|| "That geometry cannot be projected into this sketch".to_owned())?;
            vec![(
                ProjectionSource::SketchEntity {
                    sketch: feature,
                    entity,
                },
                outline,
            )]
        }
        _ => return Err(
            "Only edges, corners and faces of bodies and curves of other sketches can be projected"
                .to_owned(),
        ),
    };
    let fresh: Vec<(ProjectionSource, Outline)> = projections
        .into_iter()
        .filter(|(source, _)| !already_projected(definition, source))
        .collect();
    if fresh.is_empty() {
        return Err(match pickable {
            Pickable::Face { .. } => NOTHING_TO_PROJECT.to_owned(),
            _ => ALREADY_PROJECTED.to_owned(),
        });
    }
    let label = describe(model, sketch, pickable).unwrap_or_else(|| "Project geometry".to_owned());
    let mut transaction = document.transaction(label);
    for (source, outline) in &fresh {
        transaction.add_projection(sketch, source.clone(), outline);
    }
    Ok(transaction.finish())
}

pub fn face_projections(
    result: &SolidResult,
    body: FeatureId,
    face: FaceId,
    plane: &Plane,
) -> Vec<(ProjectionSource, Outline)> {
    let mut edges: Vec<EdgeId> = face_boundary(&result.solid, face);
    let mut seen_edges = BTreeSet::new();
    edges.retain(|edge| seen_edges.insert(*edge));
    edges
        .into_iter()
        .filter(|edge| !bodies::is_seam(&result.solid, *edge))
        .filter_map(|edge| edge_projection(result, body, edge, plane))
        .filter(|(_, outline)| !matches!(outline, Outline::Point(_)))
        .collect()
}

fn edge_projection(
    result: &SolidResult,
    body: FeatureId,
    edge: EdgeId,
    plane: &Plane,
) -> Option<(ProjectionSource, Outline)> {
    let reference = EdgeReference::capture(&result.solid, edge)?;
    let outline = edge_outline(&result.solid, edge, plane)?;
    Some((
        ProjectionSource::Edge {
            body,
            edge: reference,
        },
        outline,
    ))
}

fn already_projected(definition: &SketchFeature, source: &ProjectionSource) -> bool {
    definition
        .projections
        .values()
        .any(|existing| match (existing, source) {
            (
                ProjectionSource::Edge { body, edge },
                ProjectionSource::Edge {
                    body: other_body,
                    edge: other,
                },
            ) => body == other_body && edge.name() == other.name(),
            _ => existing == source,
        })
}

fn seen(model: &Model, sketch: FeatureId, body: FeatureId) -> Option<&SolidResult> {
    model.evaluation().body_result_seen_by(sketch, body)
}

fn seen_or_refuse(
    model: &Model,
    sketch: FeatureId,
    body: FeatureId,
) -> Result<&SolidResult, String> {
    seen(model, sketch, body).ok_or_else(|| {
        let document = model.document();
        format!(
            "{} is made after {} in the tree, so it cannot be projected into it",
            feature_name(document, body),
            feature_name(document, sketch)
        )
    })
}

fn later_geometry(document: &Document, sketch: FeatureId) -> String {
    format!(
        "That geometry is made by a feature after {} in the tree, so it cannot be projected into it",
        feature_name(document, sketch)
    )
}

fn feature_name(document: &Document, feature: FeatureId) -> String {
    document
        .feature(feature)
        .map(|feature| feature.name.clone())
        .unwrap_or_else(|| "That feature".to_owned())
}
