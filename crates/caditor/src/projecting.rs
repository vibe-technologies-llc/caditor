use std::collections::BTreeSet;

use caditor_document::{
    Document, Edit, Evaluation, Feature, FeatureId, FeatureKind, Outline, ProjectionSource,
    SectionError, SketchFeature, SolidResult, Transaction, axis_outline, datum_outline,
    edge_outline, point_outline, section_curves, sketch_outline, vertex_outline,
};
use caditor_geometry::{Aabb, Plane, Point3, Ray};
use caditor_kernel::{EdgeId, EdgeReference, FaceId};
use caditor_sketch::{Entity, EntityId};

use crate::{
    bodies, body_selection::face_boundary, datum_tools, display::Displayed, editing::ActiveSketch,
    model::Model, selection::Pickable, visibility,
};

pub type Projected = (ProjectionSource, Outline);

pub trait Surroundings {
    fn document(&self) -> &Document;

    fn evaluation(&self) -> &Evaluation;

    fn shown_sketch<'a>(&'a self, feature: &'a Feature) -> Option<Displayed<'a>>;
}

impl Surroundings for Model {
    fn document(&self) -> &Document {
        Model::document(self)
    }

    fn evaluation(&self) -> &Evaluation {
        Model::evaluation(self)
    }

    fn shown_sketch<'a>(&'a self, feature: &'a Feature) -> Option<Displayed<'a>> {
        self.displayed_sketch(feature)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProjectionError {
    #[error("The sketch being edited no longer exists")]
    SketchGone,
    #[error("Only a sketch can take projected or intersected geometry")]
    NotASketch,
    #[error("That sketch no longer exists")]
    SourceGone,
    #[error("{name} has not been computed yet")]
    NotComputed { name: String },
    #[error(
        "{feature} comes after {sketch} in the tree, so its geometry cannot be projected into it"
    )]
    FeatureAfter { feature: String, sketch: String },
    #[error("{body} is made after {sketch} in the tree, so it cannot be projected into it")]
    BodyAfter { body: String, sketch: String },
    #[error(
        "That geometry is made by a feature after {sketch} in the tree, so it cannot be projected into it"
    )]
    MadeAfter { sketch: String },
    #[error("{sketch} has no curves or points to project")]
    NothingIn { sketch: String },
    #[error("Every curve and point of {sketch} is already projected into this sketch")]
    EverythingProjected { sketch: String },
    #[error("{ALREADY_PROJECTED}")]
    AlreadyProjected,
    #[error("Every edge of that face is seen end-on from this sketch or already projected into it")]
    NothingOfFace,
    #[error("That edge cannot be projected into this sketch")]
    EdgeNotProjectable,
    #[error(
        "That corner cannot be told apart from another between the same faces, so the sketch could not follow it"
    )]
    CornerAmbiguous,
    #[error("That corner cannot be projected into this sketch")]
    CornerNotProjectable,
    #[error("That geometry cannot be projected into this sketch")]
    GeometryNotProjectable,
    #[error(
        "A sketch's own origin and axes cannot be projected; project the model's origin or axes instead"
    )]
    ReferenceNotProjected,
    #[error("Only datum axes and points can be projected; intersect a datum plane instead")]
    DatumNotProjected,
    #[error("The {axis} has no direction")]
    AxisWithoutDirection { axis: &'static str },
    #[error("That axis cannot be projected into this sketch")]
    AxisNotProjectable,
    #[error(
        "Only edges, corners and faces of bodies, curves and points of other sketches, axes, datum points and the origin can be projected"
    )]
    NotProjectable,
    #[error("{feature} comes after {sketch} in the tree, so it cannot be intersected with it")]
    DatumAfter { feature: String, sketch: String },
    #[error("{name} is not a plane")]
    NotAPlane { name: String },
    #[error("{name} is parallel to the sketch, so it does not cross it")]
    ParallelToSketch { name: String },
    #[error("The sketch plane does not cut that face")]
    FaceNotCut,
    #[error("{ALREADY_INTERSECTED}")]
    AlreadyIntersected,
    #[error("The sketch plane does not cut through {name}")]
    SectionMisses { name: String },
    #[error(
        "{name} could not be cut along the sketch plane; move the sketch slightly so it does not run exactly along a face or edge"
    )]
    SectionFailed { name: String },
    #[error(
        "Only faces and bodies, and datum and principal planes, can be intersected with the sketch"
    )]
    NotIntersectable,
}

pub const ALREADY_PROJECTED: &str = "That is already projected into this sketch";
pub const NO_SKETCH_TO_PROJECT: &str = "Highlight a curve of another sketch with the Project tool, \
                                        or choose one other sketch in the feature tree";
pub const ALREADY_INTERSECTED: &str = "That cut is already drawn in this sketch";
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
        (
            false,
            Pickable::Edge { .. }
            | Pickable::Vertex { .. }
            | Pickable::Face { .. }
            | Pickable::Axis(_)
            | Pickable::Origin,
        ) => true,
        (false, Pickable::Datum(datum)) => datum_tools::is_axis_or_point(model.document(), datum),
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
    let what = projected_words(model, sketch, pickable)?;
    let whole = match pickable {
        Pickable::SketchEntity { feature, .. } => document
            .feature(feature)
            .map(|owner| format!(", or with Shift all of {}", owner.name)),
        _ => None,
    };
    Some(format!("Project {what}{}", whole.unwrap_or_default()))
}

fn projected_words(model: &Model, sketch: FeatureId, pickable: Pickable) -> Option<String> {
    let document = model.document();
    Some(match pickable {
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
        Pickable::Datum(datum) => feature_name(document, datum),
        Pickable::Axis(axis) => format!("the {}", axis.name()),
        Pickable::Origin => "the origin".to_owned(),
        _ => return None,
    })
}

pub struct Projection {
    pub transaction: Transaction,
    pub left_out: Option<String>,
}

impl From<Transaction> for Projection {
    fn from(transaction: Transaction) -> Self {
        Self {
            transaction,
            left_out: None,
        }
    }
}

pub fn project(
    model: &Model,
    active: ActiveSketch,
    pickable: Pickable,
    whole: bool,
) -> Result<Projection, ProjectionError> {
    if active.tool.intersects() {
        return intersect(model, active.feature, pickable, whole).map(Projection::from);
    }
    if whole && let Pickable::SketchEntity { feature, .. } = pickable {
        return project_sketch(model, active.feature, feature);
    }
    project_one(model, active, pickable).map(Projection::from)
}

pub fn sketch_to_project(
    model: &Model,
    active: ActiveSketch,
    highlighted: Option<Pickable>,
    chosen_rows: &[FeatureId],
) -> Result<FeatureId, &'static str> {
    let from_view = highlighted
        .filter(|_| active.tool.projects() && !active.tool.intersects())
        .and_then(|pickable| match pickable {
            Pickable::SketchEntity { feature, .. } if feature != active.feature => Some(feature),
            _ => None,
        });
    if let Some(sketch) = from_view {
        return Ok(sketch);
    }
    let document = model.document();
    match chosen_rows {
        [row]
            if *row != active.feature
                && document
                    .feature(*row)
                    .is_some_and(|feature| feature.kind.sketch().is_some()) =>
        {
            Ok(*row)
        }
        _ => Err(NO_SKETCH_TO_PROJECT),
    }
}

pub fn project_sketch(
    model: &dyn Surroundings,
    sketch: FeatureId,
    source: FeatureId,
) -> Result<Projection, ProjectionError> {
    let document = model.document();
    let (definition, _) = sketch_plane(model, sketch)?;
    let (owner, candidates) = sketch_candidates(model, sketch, source)?;
    let total = Counted::of(candidates.iter().map(|(_, outline)| outline));
    let fresh: Vec<(ProjectionSource, Outline)> = candidates
        .into_iter()
        .filter(|(source, _)| !already_projected(definition, source))
        .collect();
    let projected = Counted::of(fresh.iter().map(|(_, outline)| outline));
    if projected.is_empty() {
        return Err(ProjectionError::EverythingProjected {
            sketch: owner.name.clone(),
        });
    }
    let mut transaction =
        document.transaction(format!("Project {} of {}", projected.words(), owner.name));
    for (source, outline) in &fresh {
        transaction.add_projection(sketch, source.clone(), outline);
    }
    let skipped = total.less(projected);
    let left_out = (!skipped.is_empty()).then(|| {
        format!(
            "{} of {} {} already projected, so {} left out",
            skipped.words(),
            owner.name,
            skipped.verb(),
            skipped.pronoun_verb()
        )
    });
    Ok(Projection {
        transaction: transaction.finish(),
        left_out,
    })
}

pub fn sketch_candidates(
    model: &dyn Surroundings,
    sketch: FeatureId,
    source: FeatureId,
) -> Result<(&Feature, Vec<Projected>), ProjectionError> {
    let document = model.document();
    refuse_later(document, source, sketch)?;
    let (_, plane) = sketch_plane(model, sketch)?;
    let owner = document
        .feature(source)
        .ok_or(ProjectionError::SourceGone)?;
    let shown = model
        .shown_sketch(owner)
        .ok_or_else(|| ProjectionError::NotComputed {
            name: owner.name.clone(),
        })?;
    let lone_points: BTreeSet<EntityId> = shown.free_points().into_iter().collect();
    let candidates: Vec<(ProjectionSource, Outline)> = shown
        .entities()
        .filter(|(id, entity)| !id.is_reference() && is_projected_whole(*id, entity, &lone_points))
        .filter_map(|(id, _)| {
            let outline = sketch_outline(&shown, id, &plane)?;
            Some((
                ProjectionSource::SketchEntity {
                    sketch: source,
                    entity: id,
                },
                outline,
            ))
        })
        .collect();
    if candidates.is_empty() {
        return Err(ProjectionError::NothingIn {
            sketch: owner.name.clone(),
        });
    }
    Ok((owner, candidates))
}

fn is_projected_whole(id: EntityId, entity: &Entity, lone_points: &BTreeSet<EntityId>) -> bool {
    !matches!(entity, Entity::Point(_)) || lone_points.contains(&id)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Counted {
    curves: usize,
    points: usize,
}

impl Counted {
    fn of<'a>(outlines: impl Iterator<Item = &'a Outline>) -> Self {
        let (points, curves): (Vec<&Outline>, Vec<&Outline>) =
            outlines.partition(|outline| matches!(outline, Outline::Point(_)));
        Self {
            curves: curves.len(),
            points: points.len(),
        }
    }

    fn is_empty(self) -> bool {
        self.curves == 0 && self.points == 0
    }

    fn less(self, other: Self) -> Self {
        Self {
            curves: self.curves.saturating_sub(other.curves),
            points: self.points.saturating_sub(other.points),
        }
    }

    fn words(self) -> String {
        let counted = |count: usize, one: &str, many: &str| match count {
            0 => None,
            1 => Some(format!("1 {one}")),
            _ => Some(format!("{count} {many}")),
        };
        [
            counted(self.curves, "curve", "curves"),
            counted(self.points, "point", "points"),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" and ")
    }

    fn is_one(self) -> bool {
        self.curves + self.points == 1
    }

    fn verb(self) -> &'static str {
        if self.is_one() { "was" } else { "were" }
    }

    fn pronoun_verb(self) -> &'static str {
        if self.is_one() { "it was" } else { "they were" }
    }
}

fn project_one(
    model: &Model,
    active: ActiveSketch,
    pickable: Pickable,
) -> Result<Transaction, ProjectionError> {
    let sketch = active.feature;
    let document = model.document();
    let (definition, plane) = sketch_plane(model, sketch)?;
    let projections = projections_of(model, sketch, &plane, pickable)?;
    let fresh: Vec<(ProjectionSource, Outline)> = projections
        .into_iter()
        .filter(|(source, _)| !already_projected(definition, source))
        .collect();
    if fresh.is_empty() {
        return Err(match pickable {
            Pickable::Face { .. } => ProjectionError::NothingOfFace,
            _ => ProjectionError::AlreadyProjected,
        });
    }
    let label = projected_words(model, sketch, pickable).map_or_else(
        || "Project geometry".to_owned(),
        |what| format!("Project {what}"),
    );
    let mut transaction = document.transaction(label);
    for (source, outline) in &fresh {
        transaction.add_projection(sketch, source.clone(), outline);
    }
    Ok(transaction.finish())
}

pub fn projections_of(
    model: &dyn Surroundings,
    sketch: FeatureId,
    plane: &Plane,
    pickable: Pickable,
) -> Result<Vec<(ProjectionSource, Outline)>, ProjectionError> {
    let document = model.document();
    Ok(match pickable {
        Pickable::Edge { body, edge } => {
            let result = seen_or_refuse(model, sketch, body)?;
            let id =
                bodies::find_edge(result, edge).ok_or_else(|| later_geometry(document, sketch))?;
            vec![
                edge_projection(result, body, id, plane)
                    .ok_or(ProjectionError::EdgeNotProjectable)?,
            ]
        }
        Pickable::Vertex { body, vertex } => {
            let result = seen_or_refuse(model, sketch, body)?;
            let id = bodies::find_vertex(result, vertex)
                .ok_or_else(|| later_geometry(document, sketch))?;
            if result.names().vertices_named(vertex.name).len() > 1 {
                return Err(ProjectionError::CornerAmbiguous);
            }
            let outline = vertex_outline(&result.solid, id, plane)
                .ok_or(ProjectionError::CornerNotProjectable)?;
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
            face_projections(result, body, &[id], plane)
        }
        Pickable::SketchEntity { feature, entity } => {
            refuse_later(document, feature, sketch)?;
            if entity.is_reference() {
                return Err(ProjectionError::ReferenceNotProjected);
            }
            let owner = document
                .feature(feature)
                .ok_or(ProjectionError::SourceGone)?;
            let source = model
                .shown_sketch(owner)
                .ok_or_else(|| ProjectionError::NotComputed {
                    name: owner.name.clone(),
                })?;
            let outline = sketch_outline(&source, entity, plane)
                .ok_or(ProjectionError::GeometryNotProjectable)?;
            vec![(
                ProjectionSource::SketchEntity {
                    sketch: feature,
                    entity,
                },
                outline,
            )]
        }
        Pickable::Datum(datum) => {
            refuse_later(document, datum, sketch)?;
            let result = datum_tools::result(model.evaluation(), datum).ok_or_else(|| {
                ProjectionError::NotComputed {
                    name: feature_name(document, datum),
                }
            })?;
            match (result.axis(), result.point()) {
                (Some(axis), _) => {
                    let reach = axis_reach(model, &axis, plane);
                    vec![(
                        ProjectionSource::DatumAxis { datum, reach },
                        axis_projection(&axis, plane, reach)?,
                    )]
                }
                (None, Some(point)) => vec![(
                    ProjectionSource::DatumPoint { datum },
                    point_outline(point, plane),
                )],
                (None, None) => return Err(ProjectionError::DatumNotProjected),
            }
        }
        Pickable::Axis(axis) => {
            let principal = axis.principal();
            let ray = principal
                .ray()
                .ok_or(ProjectionError::AxisWithoutDirection { axis: axis.name() })?;
            let reach = axis_reach(model, &ray, plane);
            vec![(
                ProjectionSource::PrincipalAxis {
                    axis: principal,
                    reach,
                },
                axis_projection(&ray, plane, reach)?,
            )]
        }
        Pickable::Origin => vec![(ProjectionSource::Origin, point_outline(Point3::ZERO, plane))],
        _ => return Err(ProjectionError::NotProjectable),
    })
}

fn axis_projection(axis: &Ray, plane: &Plane, reach: f64) -> Result<Outline, ProjectionError> {
    axis_outline(axis, plane, reach).ok_or(ProjectionError::AxisNotProjectable)
}

fn refuse_later(
    document: &Document,
    feature: FeatureId,
    sketch: FeatureId,
) -> Result<(), ProjectionError> {
    let position = |id: FeatureId| document.features().position(|other| other.id() == id);
    if position(feature) >= position(sketch) {
        return Err(ProjectionError::FeatureAfter {
            feature: feature_name(document, feature),
            sketch: feature_name(document, sketch),
        });
    }
    Ok(())
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
        .find(|(_, existing)| existing.is_same_source(source))
        .map(|(id, _)| *id)
}

fn sketch_plane(
    model: &dyn Surroundings,
    sketch: FeatureId,
) -> Result<(&SketchFeature, Plane), ProjectionError> {
    let edited = model
        .document()
        .feature(sketch)
        .ok_or(ProjectionError::SketchGone)?;
    let FeatureKind::Sketch(definition) = &edited.kind else {
        return Err(ProjectionError::NotASketch);
    };
    let plane = model
        .shown_sketch(edited)
        .map_or_else(|| definition.sketch.plane(), |displayed| displayed.plane());
    Ok((definition, plane))
}

pub fn intersect(
    model: &dyn Surroundings,
    sketch: FeatureId,
    pickable: Pickable,
    whole: bool,
) -> Result<Transaction, ProjectionError> {
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
                return Err(ProjectionError::FaceNotCut);
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
                return Err(ProjectionError::DatumAfter {
                    feature: feature_name(document, datum),
                    sketch: feature_name(document, sketch),
                });
            }
            let datum_plane = datum_tools::result(model.evaluation(), datum)
                .and_then(|result| result.plane())
                .ok_or_else(|| ProjectionError::NotAPlane {
                    name: feature_name(document, datum),
                })?;
            let reach = datum_reach(model, &datum_plane, &plane);
            let outline = datum_outline(&datum_plane, &plane, reach).ok_or_else(|| {
                ProjectionError::ParallelToSketch {
                    name: feature_name(document, datum),
                }
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
                ProjectionError::ParallelToSketch {
                    name: format!("The {}", principal.name()),
                }
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
        _ => return Err(ProjectionError::NotIntersectable),
    };
    let fresh: Vec<(ProjectionSource, Outline)> = cuts
        .into_iter()
        .filter(|(source, _)| !already_projected(definition, source))
        .collect();
    if fresh.is_empty() {
        return Err(ProjectionError::AlreadyIntersected);
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

fn section_refusal(document: &Document, body: FeatureId, error: &SectionError) -> ProjectionError {
    let name = feature_name(document, body);
    match error {
        SectionError::Misses => ProjectionError::SectionMisses { name },
        SectionError::HalfSpace(_) | SectionError::Boolean(_) => {
            log::warn!("{name} could not be cut along the sketch plane: {error}");
            ProjectionError::SectionFailed { name }
        }
    }
}

fn datum_reach(model: &dyn Surroundings, datum: &Plane, plane: &Plane) -> f64 {
    let foot = datum_outline(datum, plane, 1.0).and_then(|outline| middle(&outline, plane));
    reach_about(model, foot)
}

fn axis_reach(model: &dyn Surroundings, axis: &Ray, plane: &Plane) -> f64 {
    let foot = axis_outline(axis, plane, 1.0).and_then(|outline| middle(&outline, plane));
    reach_about(model, foot)
}

fn middle(outline: &Outline, plane: &Plane) -> Option<Point3> {
    match outline {
        Outline::Line { start, end } => Some(plane.to_world((*start + *end) / 2.0)),
        _ => None,
    }
}

fn reach_about(model: &dyn Surroundings, foot: Option<Point3>) -> f64 {
    let document = model.document();
    let evaluation = model.evaluation();
    let bounds = evaluation
        .bodies()
        .map(|(body, _)| body)
        .filter(|body| visibility::is_shown(document, *body))
        .filter_map(|body| bodies::shown(evaluation, body)?.solid.bounding_box())
        .reduce(Aabb::union);
    match (bounds, foot) {
        (Some(bounds), Some(foot)) => {
            (foot.distance(bounds.center()) + bounds.diagonal() / 2.0) * DATUM_REACH_MARGIN
        }
        _ => DEFAULT_DATUM_REACH,
    }
    .max(DEFAULT_DATUM_REACH / 10.0)
}

fn seen(model: &dyn Surroundings, sketch: FeatureId, body: FeatureId) -> Option<&SolidResult> {
    model.evaluation().body_result_seen_by(sketch, body)
}

fn seen_or_refuse(
    model: &dyn Surroundings,
    sketch: FeatureId,
    body: FeatureId,
) -> Result<&SolidResult, ProjectionError> {
    seen(model, sketch, body).ok_or_else(|| {
        let document = model.document();
        ProjectionError::BodyAfter {
            body: feature_name(document, body),
            sketch: feature_name(document, sketch),
        }
    })
}

fn later_geometry(document: &Document, sketch: FeatureId) -> ProjectionError {
    ProjectionError::MadeAfter {
        sketch: feature_name(document, sketch),
    }
}

fn feature_name(document: &Document, feature: FeatureId) -> String {
    document
        .body_label(feature)
        .unwrap_or_else(|| "That feature".to_owned())
}
