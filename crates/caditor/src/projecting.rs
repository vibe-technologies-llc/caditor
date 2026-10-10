use std::collections::BTreeSet;

use caditor_document::{
    Document, Edit, FeatureId, FeatureKind, Outline, ProjectionSource, SectionError, SketchFeature,
    SolidResult, Transaction, datum_outline, edge_outline, section_curves, sketch_outline,
    vertex_outline,
};
use caditor_geometry::{Aabb, Plane};
use caditor_kernel::{EdgeId, EdgeReference, FaceId};
use caditor_sketch::EntityId;

use crate::{
    bodies, body_selection::face_boundary, datum_tools, editing::ActiveSketch, model::Model,
    selection::Pickable, visibility,
};

pub const ALREADY_PROJECTED: &str = "That is already projected into this sketch";
const NOTHING_TO_PROJECT: &str =
    "Every edge of that face is seen end-on from this sketch or already projected into it";
pub const ALREADY_INTERSECTED: &str = "That cut is already drawn in this sketch";
pub const FACE_NOT_CUT: &str = "The sketch plane does not cut that face";
pub const NO_FACE_HIGHLIGHTED: &str =
    "Choose the Intersect tool and highlight a face of a body first";
const DEFAULT_DATUM_REACH: f64 = 50.0;
const DATUM_REACH_MARGIN: f64 = 1.1;

pub fn projectable(model: &Model, active: ActiveSketch, pickable: Pickable) -> bool {
    match (active.tool.intersects(), pickable) {
        (true, Pickable::Face { .. }) => true,
        (true, Pickable::Datum(datum)) => datum_tools::is_plane(model.document(), datum),
        (true, Pickable::Plane(_)) => true,
        (true, _) => false,
        (false, Pickable::Edge { .. } | Pickable::Vertex { .. } | Pickable::Face { .. }) => true,
        (false, Pickable::SketchEntity { feature, entity }) => {
            feature != active.feature && !entity.is_reference()
        }
        (false, _) => false,
    }
}

pub fn describe(model: &Model, active: ActiveSketch, pickable: Pickable) -> Option<String> {
    let sketch = active.feature;
    let document = model.document();
    if active.tool.intersects() {
        return match pickable {
            Pickable::Face { body, face } => {
                let result = seen(model, sketch, body)?;
                Some(format!(
                    "Intersect {}, or with Shift all of {}",
                    bodies::describe_face(document, result, face),
                    feature_name(document, body)
                ))
            }
            Pickable::Datum(datum) => Some(format!("Intersect {}", feature_name(document, datum))),
            Pickable::Plane(plane) => Some(format!("Intersect the {}", plane.name())),
            _ => None,
        };
    }
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
    active: ActiveSketch,
    pickable: Pickable,
    whole: bool,
) -> Result<Transaction, String> {
    if active.tool.intersects() {
        return intersect(model, active.feature, pickable, whole);
    }
    let sketch = active.feature;
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
            face_projections(result, body, &[id], &plane)
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
    let label = describe(model, active, pickable).unwrap_or_else(|| "Project geometry".to_owned());
    let mut transaction = document.transaction(label);
    for (source, outline) in &fresh {
        transaction.add_projection(sketch, source.clone(), outline);
    }
    Ok(transaction.finish())
}

pub fn face_projections(
    result: &SolidResult,
    body: FeatureId,
    faces: &[FaceId],
    plane: &Plane,
) -> Vec<(ProjectionSource, Outline)> {
    let mut edges: Vec<EdgeId> = faces
        .iter()
        .flat_map(|face| face_boundary(&result.solid, *face))
        .collect();
    let mut seen_edges = BTreeSet::new();
    edges.retain(|edge| seen_edges.insert(*edge));
    edges
        .into_iter()
        .filter(|edge| !bodies::is_seam(&result.solid, *edge))
        .filter_map(|edge| edge_projection(result, body, edge, plane))
        .filter(|(_, outline)| !matches!(outline, Outline::Point(_)))
        .collect()
}

pub fn edge_projection(
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
    existing_projection(definition, source).is_some()
}

pub fn existing_projection(
    definition: &SketchFeature,
    source: &ProjectionSource,
) -> Option<EntityId> {
    definition
        .projections
        .iter()
        .find(|(_, existing)| match (*existing, source) {
            (
                ProjectionSource::Edge { body, edge },
                ProjectionSource::Edge {
                    body: other_body,
                    edge: other,
                },
            )
            | (
                ProjectionSource::Section { body, edge },
                ProjectionSource::Section {
                    body: other_body,
                    edge: other,
                },
            ) => body == other_body && edge.name() == other.name(),
            (
                ProjectionSource::DatumPlane { datum, .. },
                ProjectionSource::DatumPlane { datum: other, .. },
            ) => datum == other,
            (
                ProjectionSource::PrincipalPlane { plane, .. },
                ProjectionSource::PrincipalPlane { plane: other, .. },
            ) => plane == other,
            _ => *existing == source,
        })
        .map(|(id, _)| *id)
}

fn sketch_plane(model: &Model, sketch: FeatureId) -> Result<(&SketchFeature, Plane), String> {
    let edited = model
        .document()
        .feature(sketch)
        .ok_or_else(|| "The sketch being edited no longer exists".to_owned())?;
    let FeatureKind::Sketch(definition) = &edited.kind else {
        return Err("Only a sketch can take intersected geometry".to_owned());
    };
    let plane = model
        .displayed_sketch(edited)
        .map_or_else(|| definition.sketch.plane(), |displayed| displayed.plane());
    Ok((definition, plane))
}

fn intersect(
    model: &Model,
    sketch: FeatureId,
    pickable: Pickable,
    whole: bool,
) -> Result<Transaction, String> {
    let document = model.document();
    let (definition, plane) = sketch_plane(model, sketch)?;
    let (cuts, construction, label) = match pickable {
        Pickable::Face { body, face } => {
            let result = seen_or_refuse(model, sketch, body)?;
            let id =
                bodies::find_face(result, face).ok_or_else(|| later_geometry(document, sketch))?;
            let name = result
                .solid
                .face(id)
                .map(caditor_kernel::Face::name)
                .ok_or_else(|| later_geometry(document, sketch))?;
            let curves = section_curves(body, &result.solid, &plane, sketch)
                .map_err(|error| section_refusal(document, body, &error))?;
            let cuts: Vec<(ProjectionSource, Outline)> = curves
                .into_iter()
                .filter(|curve| whole || curve.face == name)
                .map(|curve| (curve.source, curve.outline))
                .collect();
            if cuts.is_empty() {
                return Err(FACE_NOT_CUT.to_owned());
            }
            let label = if whole {
                format!("Intersect {}", feature_name(document, body))
            } else {
                format!(
                    "Intersect {}",
                    bodies::describe_face(document, result, face)
                )
            };
            (cuts, false, label)
        }
        Pickable::Datum(datum) => {
            let position = |id: FeatureId| document.features().position(|other| other.id() == id);
            if position(datum) >= position(sketch) {
                return Err(format!(
                    "{} comes after {} in the tree, so it cannot be intersected with it",
                    feature_name(document, datum),
                    feature_name(document, sketch)
                ));
            }
            let datum_plane = datum_tools::result(model.evaluation(), datum)
                .and_then(|result| result.plane())
                .ok_or_else(|| format!("{} is not a plane", feature_name(document, datum)))?;
            let reach = datum_reach(model, &datum_plane, &plane);
            let outline = datum_outline(&datum_plane, &plane, reach).ok_or_else(|| {
                format!(
                    "{} is parallel to the sketch, so it does not cross it",
                    feature_name(document, datum)
                )
            })?;
            let label = format!("Intersect {}", feature_name(document, datum));
            (
                vec![(ProjectionSource::DatumPlane { datum, reach }, outline)],
                true,
                label,
            )
        }
        Pickable::Plane(principal) => {
            let reach = datum_reach(model, &principal.plane(), &plane);
            let outline = datum_outline(&principal.plane(), &plane, reach).ok_or_else(|| {
                format!(
                    "The {} is parallel to the sketch, so it does not cross it",
                    principal.name()
                )
            })?;
            (
                vec![(
                    ProjectionSource::PrincipalPlane {
                        plane: principal,
                        reach,
                    },
                    outline,
                )],
                true,
                format!("Intersect the {}", principal.name()),
            )
        }
        _ => {
            return Err(
                "Only faces and bodies, and datum and principal planes, can be intersected with \
                 the sketch"
                    .to_owned(),
            );
        }
    };
    let fresh: Vec<(ProjectionSource, Outline)> = cuts
        .into_iter()
        .filter(|(source, _)| !already_projected(definition, source))
        .collect();
    if fresh.is_empty() {
        return Err(ALREADY_INTERSECTED.to_owned());
    }
    let mut transaction = document.transaction(label);
    for (source, outline) in &fresh {
        let id = transaction.add_projection(sketch, source.clone(), outline);
        if construction {
            transaction.edit(Edit::SetSketchConstruction {
                feature: sketch,
                id,
                construction: true,
            });
        }
    }
    Ok(transaction.finish())
}

fn section_refusal(document: &Document, body: FeatureId, error: &SectionError) -> String {
    let name = feature_name(document, body);
    match error {
        SectionError::Misses => format!("The sketch plane does not cut through {name}"),
        SectionError::HalfSpace(_) | SectionError::Boolean(_) => {
            log::warn!("{name} could not be cut along the sketch plane: {error}");
            format!(
                "{name} could not be cut along the sketch plane; move the sketch slightly so it \
                 does not run exactly along a face or edge"
            )
        }
    }
}

fn datum_reach(model: &Model, datum: &Plane, plane: &Plane) -> f64 {
    let document = model.document();
    let evaluation = model.evaluation();
    let bounds = evaluation
        .bodies()
        .map(|(body, _)| body)
        .filter(|body| visibility::is_shown(document, *body))
        .filter_map(|body| bodies::shown(evaluation, body)?.solid.bounding_box())
        .reduce(Aabb::union);
    let foot = datum_outline(datum, plane, 1.0).and_then(|outline| match outline {
        Outline::Line { start, end } => Some(plane.to_world((start + end) / 2.0)),
        _ => None,
    });
    match (bounds, foot) {
        (Some(bounds), Some(foot)) => {
            (foot.distance(bounds.center()) + bounds.diagonal() / 2.0) * DATUM_REACH_MARGIN
        }
        _ => DEFAULT_DATUM_REACH,
    }
    .max(DEFAULT_DATUM_REACH / 10.0)
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
