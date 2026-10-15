use caditor_document::{
    Document, Evaluation, Feature, FeatureState, PrincipalPlane, SketchRegion, SolidResult,
    face_plane,
};
use caditor_file::{PngExportError, png_bytes};
use caditor_geometry::{Aabb, Plane, Point2, Point3, Vector3};
use caditor_kernel::{
    Accuracy, FaceForm, MeasureError, RegionMesh, extent, face_area, face_form, interruptible,
};
use caditor_render::{
    Background, GraphicsSettings, ImageError, ImageRequest, OffscreenRenderer,
    RenderError as AdapterError, SurfaceSize, Viewpoint,
};
use caditor_sketch::{Entity, FitSpacing, Sketch, SplineKind};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::{
    bodies,
    commands::StandardView,
    feature_tree::kind_words,
    image_export::RenderedRows,
    mcp::{
        references::{self, Named, body_label, entity_name, face_key_text},
        session::{Cancelled, Session, Work},
        tools::{Answer, ToolError},
    },
    model::display_name,
    selection, sketch_placement,
    sketch_status::{SketchSummary, up_to_date_result},
    snapshot::{self, Unmeshed},
};

pub const SMALLEST_SIDE: u32 = 64;
pub const LARGEST_SIDE: u32 = 2048;
const DEFAULT_SIZE: SurfaceSize = SurfaceSize {
    width: 800,
    height: 600,
};
const MOST_FACES: usize = 300;
const DECIMALS: f64 = 1e6;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Nothing {}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OfBody {
    pub body: Named,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OfSketch {
    pub sketch: Named,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewName {
    Home,
    Isometric,
    Front,
    Top,
    Right,
    Back,
    Bottom,
    Left,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Picture {
    pub view: Option<ViewName>,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error(
        "a picture is {SMALLEST_SIDE} to {LARGEST_SIDE} pixels a side, so {0} pixels cannot be drawn"
    )]
    Size(u32),
    #[error("the bodies could not be prepared for drawing: {0}")]
    Unmeshed(Unmeshed),
    #[error("no graphics adapter can draw a picture here: {0}")]
    NoAdapter(#[source] AdapterError),
    #[error("the picture could not be drawn: {0}")]
    Drawing(#[source] ImageError),
    #[error("the picture could not be encoded: {0}")]
    Encoding(#[source] PngExportError<ImageError>),
}

pub fn rounded(value: f64) -> Value {
    let kept = (value * DECIMALS).round() / DECIMALS;
    json!(if kept == 0.0 { 0.0 } else { kept })
}

pub fn point3(point: Point3) -> Value {
    json!([rounded(point.x), rounded(point.y), rounded(point.z)])
}

pub fn vector3(vector: Vector3) -> Value {
    json!([rounded(vector.x), rounded(vector.y), rounded(vector.z)])
}

pub fn point2(point: Point2) -> Value {
    json!([rounded(point.x), rounded(point.y)])
}

pub fn plane(plane: &Plane) -> Value {
    json!({
        "origin": point3(plane.origin()),
        "x_axis": vector3(plane.x_axis()),
        "y_axis": vector3(plane.y_axis()),
        "normal": vector3(plane.normal()),
    })
}

pub struct StateWords {
    pub state: &'static str,
    pub reason: Option<String>,
    pub remedy: Option<String>,
}

impl StateWords {
    fn plain(state: &'static str) -> Self {
        Self {
            state,
            reason: None,
            remedy: None,
        }
    }

    pub fn is_problem(&self) -> bool {
        self.reason.is_some()
    }

    pub fn into_json(self, into: &mut Map<String, Value>) {
        into.insert("state".to_owned(), json!(self.state));
        if let Some(reason) = self.reason {
            into.insert("reason".to_owned(), json!(reason));
        }
        if let Some(remedy) = self.remedy {
            into.insert("remedy".to_owned(), json!(remedy));
        }
    }
}

pub fn state_of(document: &Document, evaluation: &Evaluation, feature: &Feature) -> StateWords {
    if feature.suppressed {
        return StateWords::plain("suppressed");
    }
    if document.is_rolled_back(feature.id()) {
        return StateWords::plain("below the rollback bar");
    }
    match evaluation.feature(feature.id()).map(|status| &status.state) {
        Some(FeatureState::UpToDate) => StateWords::plain("up to date"),
        Some(FeatureState::Failed(error)) => StateWords {
            state: "failed",
            reason: Some(error.reason.clone()),
            remedy: Some(error.remedy.clone()),
        },
        Some(FeatureState::Waiting(missing)) => StateWords {
            state: "waiting",
            reason: Some(missing.waiting_words().to_owned()),
            remedy: Some(missing.prompt(document)),
        },
        Some(FeatureState::Outdated) => StateWords::plain("outdated"),
        Some(FeatureState::Suppressed) => StateWords::plain("suppressed"),
        Some(FeatureState::RolledBack) => StateWords::plain("below the rollback bar"),
        None => StateWords::plain("not computed"),
    }
}

pub fn kind_word(feature: &Feature) -> &'static str {
    kind_words(&feature.kind)
        .first()
        .copied()
        .unwrap_or("feature")
}

pub fn feature_brief(feature: &Feature) -> Value {
    json!({ "id": feature.id().raw(), "name": feature.name })
}

pub fn problems(document: &Document, evaluation: &Evaluation) -> Vec<(u64, String, StateWords)> {
    document
        .features()
        .filter_map(|feature| {
            let words = state_of(document, evaluation, feature);
            words
                .is_problem()
                .then(|| (feature.id().raw(), feature.name.clone(), words))
        })
        .collect()
}

pub fn model(session: &mut Session, work: &Work<'_>) -> Result<Map<String, Value>, ToolError> {
    session.evaluate(work)?;
    let unprotected = session.unprotected().map(ToString::to_string);
    let document = session.document();
    let evaluation = session.evaluation();
    let failing: Vec<Value> = problems(document, evaluation)
        .into_iter()
        .map(|(id, name, words)| {
            let mut entry = Map::new();
            entry.insert("id".to_owned(), json!(id));
            entry.insert("name".to_owned(), json!(name));
            words.into_json(&mut entry);
            Value::Object(entry)
        })
        .collect();
    let folders: Vec<String> = session
        .folders()
        .iter()
        .map(|folder| folder.display().to_string())
        .collect();
    let mut summary = Map::new();
    summary.insert(
        "file".to_owned(),
        json!(session.model().display().to_string()),
    );
    summary.insert(
        "name".to_owned(),
        json!(display_name(Some(session.model()))),
    );
    summary.insert("writable_folders".to_owned(), json!(folders));
    summary.insert("unsaved_changes".to_owned(), json!(session.is_unsaved()));
    summary.insert("undo".to_owned(), json!(session.undo_label()));
    summary.insert("redo".to_owned(), json!(session.redo_label()));
    summary.insert("features".to_owned(), json!(document.features().len()));
    summary.insert("parameters".to_owned(), json!(document.parameters().len()));
    summary.insert("bodies".to_owned(), json!(evaluation.bodies().count()));
    summary.insert("failing".to_owned(), Value::Array(failing));
    summary.insert(
        "units".to_owned(),
        json!("lengths in mm, angles in degrees"),
    );
    if !session.load_issues().is_empty() {
        summary.insert("load_issues".to_owned(), json!(session.load_issues()));
    }
    if let Some(unprotected) = unprotected {
        summary.insert(
            "not_protected".to_owned(),
            json!(format!(
                "Unsaved changes are not protected against a crash: {unprotected}. Save to keep \
                 them safe."
            )),
        );
    }
    Ok(summary)
}

pub fn features(session: &mut Session, work: &Work<'_>) -> Result<Map<String, Value>, ToolError> {
    session.evaluate(work)?;
    let document = session.document();
    let evaluation = session.evaluation();
    let listed: Vec<Value> = document
        .features()
        .map(|feature| {
            let mut entry = Map::new();
            entry.insert("id".to_owned(), json!(feature.id().raw()));
            entry.insert("name".to_owned(), json!(feature.name));
            entry.insert("kind".to_owned(), json!(kind_word(feature)));
            state_of(document, evaluation, feature).into_json(&mut entry);
            if feature.hidden {
                entry.insert("hidden".to_owned(), json!(true));
            }
            if let Some(group) = &feature.group {
                entry.insert("folder".to_owned(), json!(group));
            }
            if feature.kind.sketch().is_some() {
                let summary = SketchSummary::of(evaluation, feature.id());
                entry.insert("sketch_status".to_owned(), json!(summary.status_text()));
            }
            Value::Object(entry)
        })
        .collect();
    let mut answer = Map::new();
    answer.insert("features".to_owned(), Value::Array(listed));
    Ok(answer)
}

pub fn parameters(session: &Session) -> Map<String, Value> {
    let document = session.document();
    let values = session.parameters();
    let listed: Vec<Value> = document
        .parameters()
        .iter()
        .map(|parameter| {
            let mut entry = Map::new();
            entry.insert("id".to_owned(), json!(parameter.id().raw()));
            entry.insert("name".to_owned(), json!(parameter.name));
            entry.insert(
                "expression".to_owned(),
                json!(document.expression_text(&parameter.expression)),
            );
            match values.get(parameter.id()) {
                Some(Ok(value)) => {
                    entry.insert("value".to_owned(), json!(value.to_string()));
                }
                Some(Err(error)) => {
                    entry.insert("error".to_owned(), json!(error.to_string()));
                }
                None => {}
            }
            if !parameter.note.is_empty() {
                entry.insert("note".to_owned(), json!(parameter.note));
            }
            let users = document.parameter_users(parameter.id());
            if !users.is_empty() {
                entry.insert("used_by".to_owned(), json!(users));
            }
            Value::Object(entry)
        })
        .collect();
    let mut answer = Map::new();
    answer.insert("parameters".to_owned(), Value::Array(listed));
    answer
}

pub fn bodies(session: &mut Session, work: &Work<'_>) -> Result<Map<String, Value>, ToolError> {
    session.evaluate(work)?;
    let document = session.document();
    let evaluation = session.evaluation();
    let standing: Vec<_> = evaluation.bodies().collect();
    let count = standing.len();
    let mut listed = Vec::with_capacity(count);
    for (index, (body, last)) in standing.into_iter().enumerate() {
        let label = body_label(document, body);
        work.progress(&format!("Measuring {label} ({} of {count})", index + 1));
        let mut entry = Map::new();
        entry.insert("id".to_owned(), json!(body.raw()));
        entry.insert("name".to_owned(), json!(label));
        if let Some(feature) = document.feature(last) {
            entry.insert("last_changed_by".to_owned(), feature_brief(feature));
        }
        if let Some(result) = bodies::shown(evaluation, body) {
            measure_body(result, work, &mut entry)?;
        }
        listed.push(Value::Object(entry));
    }
    let mut answer = Map::new();
    answer.insert("bodies".to_owned(), Value::Array(listed));
    Ok(answer)
}

fn measure_body(
    result: &SolidResult,
    work: &Work<'_>,
    entry: &mut Map<String, Value>,
) -> Result<(), Cancelled> {
    let measured = interruptible(work.interrupt(), || result.exact_mass());
    match measured {
        Ok(mass) => {
            let properties = mass.properties;
            entry.insert("volume".to_owned(), rounded(properties.volume));
            entry.insert("surface_area".to_owned(), rounded(properties.area));
            entry.insert("centre_of_mass".to_owned(), point3(properties.centroid));
            entry.insert(
                "exact".to_owned(),
                json!(matches!(mass.accuracy, Accuracy::Exact)),
            );
        }
        Err(MeasureError::Cancelled(_)) => return Err(Cancelled),
        Err(error) => {
            entry.insert(
                "mass_properties".to_owned(),
                json!(format!("could not be worked out: {error}")),
            );
        }
    }
    match interruptible(work.interrupt(), || extent(&result.solid)) {
        Ok(Some(bounds)) => {
            entry.insert("bounding_box".to_owned(), bounding_box(&bounds));
        }
        Ok(None) => {}
        Err(MeasureError::Cancelled(_)) => return Err(Cancelled),
        Err(error) => {
            entry.insert(
                "bounding_box".to_owned(),
                json!(format!("could not be worked out: {error}")),
            );
        }
    }
    if work.is_cancelled() {
        return Err(Cancelled);
    }
    Ok(())
}

fn bounding_box(bounds: &Aabb) -> Value {
    json!({
        "min": point3(bounds.min()),
        "max": point3(bounds.max()),
        "size": vector3(bounds.max() - bounds.min()),
    })
}

pub fn faces(
    session: &mut Session,
    work: &Work<'_>,
    of: &OfBody,
) -> Result<Map<String, Value>, ToolError> {
    session.evaluate(work)?;
    let document = session.document();
    let evaluation = session.evaluation();
    let body = references::body(document, evaluation, &of.body)?;
    let shown = bodies::shown(evaluation, body)
        .ok_or_else(|| references::ReferenceError::NoBody(of.body.clone()))?;
    let keys = bodies::face_keys(&shown.solid);
    let total = keys.len();
    let mut listed = Vec::with_capacity(total.min(MOST_FACES));
    for (face, key) in keys.into_iter().take(MOST_FACES) {
        let mut entry = Map::new();
        entry.insert("face".to_owned(), json!(face_key_text(key)));
        entry.insert(
            "description".to_owned(),
            json!(bodies::describe_face_id(document, shown, face)),
        );
        describe_surface(shown, face, &mut entry);
        match interruptible(work.interrupt(), || face_area(&shown.solid, face)) {
            Ok(Some(area)) => {
                entry.insert("area".to_owned(), rounded(area));
            }
            Ok(None) => {}
            Err(MeasureError::Cancelled(_)) => return Err(Cancelled.into()),
            Err(_) => {}
        }
        listed.push(Value::Object(entry));
    }
    let mut answer = Map::new();
    answer.insert("body".to_owned(), json!(body_label(document, body)));
    answer.insert("faces".to_owned(), Value::Array(listed));
    if total > MOST_FACES {
        answer.insert(
            "left_out".to_owned(),
            json!(format!(
                "{} more faces are not listed",
                total.saturating_sub(MOST_FACES)
            )),
        );
    }
    Ok(answer)
}

fn describe_surface(
    shown: &SolidResult,
    face: caditor_kernel::FaceId,
    entry: &mut Map<String, Value>,
) {
    if let Some(flat) = face_plane(&shown.solid, face) {
        entry.insert("surface".to_owned(), json!("plane"));
        entry.insert("normal".to_owned(), vector3(flat.normal()));
        entry.insert("point".to_owned(), point3(flat.origin()));
        return;
    }
    match face_form(&shown.solid, face) {
        Ok(FaceForm::Cylinder { axis, radius }) => {
            entry.insert("surface".to_owned(), json!("cylinder"));
            entry.insert("radius".to_owned(), rounded(radius));
            entry.insert("axis_point".to_owned(), point3(axis.origin));
            entry.insert("axis_direction".to_owned(), vector3(axis.direction));
        }
        Ok(FaceForm::Cone {
            apex, half_angle, ..
        }) => {
            entry.insert("surface".to_owned(), json!("cone"));
            entry.insert("apex".to_owned(), point3(apex));
            entry.insert("half_angle".to_owned(), rounded(half_angle.to_degrees()));
        }
        Ok(FaceForm::Sphere { center, radius }) => {
            entry.insert("surface".to_owned(), json!("sphere"));
            entry.insert("center".to_owned(), point3(center));
            entry.insert("radius".to_owned(), rounded(radius));
        }
        Ok(FaceForm::Torus {
            major_radius,
            minor_radius,
            ..
        }) => {
            entry.insert("surface".to_owned(), json!("torus"));
            entry.insert("major_radius".to_owned(), rounded(major_radius));
            entry.insert("minor_radius".to_owned(), rounded(minor_radius));
        }
        Ok(FaceForm::Plane { normal, origin }) => {
            entry.insert("surface".to_owned(), json!("plane"));
            entry.insert("normal".to_owned(), vector3(normal));
            entry.insert("point".to_owned(), point3(origin));
        }
        Ok(FaceForm::Surface) | Err(_) => {
            entry.insert("surface".to_owned(), json!("freeform"));
        }
    }
}

pub fn sketch(
    session: &mut Session,
    work: &Work<'_>,
    of: &OfSketch,
) -> Result<Map<String, Value>, ToolError> {
    session.evaluate(work)?;
    let document = session.document();
    let evaluation = session.evaluation();
    let (feature, definition) = references::sketch(document, &of.sketch)?;
    let result = up_to_date_result(evaluation, feature.id());
    let shown = result.map_or(definition, |result| &result.geometry);
    let summary = SketchSummary::of(evaluation, feature.id());

    let mut answer = Map::new();
    answer.insert("id".to_owned(), json!(feature.id().raw()));
    answer.insert("name".to_owned(), json!(feature.name));
    answer.insert("plane".to_owned(), plane(&shown.plane()));
    answer.insert(
        "lies_on".to_owned(),
        json!(lies_on(document, feature, definition)),
    );
    answer.insert("status".to_owned(), json!(summary.status_text()));
    if let Some(result) = result {
        answer.insert(
            "degrees_of_freedom".to_owned(),
            json!(result.solution.degrees_of_freedom()),
        );
        let redundant: Vec<String> = result
            .solution
            .redundancies()
            .iter()
            .map(|redundancy| definition.describe_constraint(redundancy.constraint))
            .collect();
        if !redundant.is_empty() {
            answer.insert("redundant".to_owned(), json!(redundant));
        }
        if !result.open_ends.is_empty() {
            let ends: Vec<Value> = result.open_ends.iter().map(|id| entity_name(*id)).collect();
            answer.insert("open_ends".to_owned(), Value::Array(ends));
        }
    }
    if let Some(FeatureState::Failed(error)) =
        evaluation.feature(feature.id()).map(|status| &status.state)
    {
        answer.insert("problem".to_owned(), json!(error.reason));
        answer.insert("remedy".to_owned(), json!(error.remedy));
        let conflicts: Vec<String> = error
            .constraints
            .iter()
            .map(|constraint| definition.describe_constraint(*constraint))
            .collect();
        if !conflicts.is_empty() {
            answer.insert("conflicting".to_owned(), json!(conflicts));
        }
    }
    answer.insert("entities".to_owned(), Value::Array(entities(shown)));
    if let Some(regions) = selection::sketch_regions(evaluation, feature.id()) {
        answer.insert("regions".to_owned(), Value::Array(regions_listed(regions)));
    }
    answer.insert(
        "constraints".to_owned(),
        Value::Array(constraints(
            document,
            definition,
            result.map(|r| &r.solution),
        )),
    );
    Ok(answer)
}

fn regions_listed(regions: &[SketchRegion]) -> Vec<Value> {
    regions
        .iter()
        .map(|region| {
            let mut entry = Map::new();
            if let Some(inside) = region.mesh.as_ref().and_then(RegionMesh::anchor) {
                entry.insert("inside".to_owned(), point2(inside));
            }
            entry.insert("area".to_owned(), rounded(region.region.area()));
            entry.insert("holes".to_owned(), json!(region.region.holes().len()));
            Value::Object(entry)
        })
        .collect()
}

fn lies_on(document: &Document, feature: &Feature, definition: &Sketch) -> String {
    if let Some(attachment) = feature.kind.attachment() {
        return sketch_placement::describe(document, attachment);
    }
    PrincipalPlane::ALL
        .into_iter()
        .find(|principal| principal.plane() == definition.plane())
        .map_or_else(
            || "a plane of its own".to_owned(),
            |principal| principal.name().to_owned(),
        )
}

fn entities(sketch: &Sketch) -> Vec<Value> {
    sketch
        .entities()
        .map(|(id, entity)| {
            let mut entry = Map::new();
            entry.insert("id".to_owned(), entity_name(id));
            entry.insert("label".to_owned(), json!(sketch.entity_label(id)));
            match entity {
                Entity::Point(at) => {
                    entry.insert("kind".to_owned(), json!("point"));
                    entry.insert("at".to_owned(), point2(*at));
                }
                Entity::Line { start, end } => {
                    entry.insert("kind".to_owned(), json!("line"));
                    entry.insert("start".to_owned(), entity_name(*start));
                    entry.insert("end".to_owned(), entity_name(*end));
                    if let Some((from, to)) = sketch.line_endpoints(id) {
                        entry.insert("from".to_owned(), point2(from));
                        entry.insert("to".to_owned(), point2(to));
                    }
                }
                Entity::Circle { center, radius } => {
                    entry.insert("kind".to_owned(), json!("circle"));
                    entry.insert("center".to_owned(), entity_name(*center));
                    if let Some(at) = sketch.point(*center) {
                        entry.insert("at".to_owned(), point2(at));
                    }
                    entry.insert("radius".to_owned(), rounded(*radius));
                }
                Entity::Arc { center, start, end } => {
                    entry.insert("kind".to_owned(), json!("arc"));
                    entry.insert("center".to_owned(), entity_name(*center));
                    entry.insert("start".to_owned(), entity_name(*start));
                    entry.insert("end".to_owned(), entity_name(*end));
                }
                Entity::Spline { points, kind } => {
                    entry.insert("kind".to_owned(), json!("spline"));
                    let points: Vec<Value> = points.iter().map(|id| entity_name(*id)).collect();
                    entry.insert("points".to_owned(), Value::Array(points));
                    let form = match kind {
                        SplineKind::Control { .. } => "control",
                        SplineKind::Fit { .. } => "fit",
                        SplineKind::Conic { .. } => "conic",
                        SplineKind::Piece { .. } => "piece",
                    };
                    entry.insert("form".to_owned(), json!(form));
                    if let SplineKind::Conic { rho } = kind {
                        entry.insert("rho".to_owned(), rounded(*rho));
                    }
                    if let SplineKind::Fit { spacing, .. } = kind {
                        let spacing = match spacing {
                            FitSpacing::Even => "even",
                            FitSpacing::Centripetal => "centripetal",
                        };
                        entry.insert("spacing".to_owned(), json!(spacing));
                    }
                }
                Entity::Ellipse {
                    center,
                    major,
                    minor_radius,
                } => {
                    entry.insert("kind".to_owned(), json!("ellipse"));
                    entry.insert("center".to_owned(), entity_name(*center));
                    entry.insert("major".to_owned(), entity_name(*major));
                    entry.insert("minor_radius".to_owned(), rounded(*minor_radius));
                }
                Entity::EllipticalArc {
                    center,
                    major,
                    minor_radius,
                    start,
                    end,
                } => {
                    entry.insert("kind".to_owned(), json!("elliptical arc"));
                    entry.insert("center".to_owned(), entity_name(*center));
                    entry.insert("major".to_owned(), entity_name(*major));
                    entry.insert("minor_radius".to_owned(), rounded(*minor_radius));
                    entry.insert("start".to_owned(), entity_name(*start));
                    entry.insert("end".to_owned(), entity_name(*end));
                }
            }
            if sketch.is_construction(id) {
                entry.insert("construction".to_owned(), json!(true));
            }
            if sketch.is_projected(id) {
                entry.insert("projected".to_owned(), json!(true));
            }
            Value::Object(entry)
        })
        .collect()
}

fn constraints(
    document: &Document,
    sketch: &Sketch,
    solution: Option<&caditor_sketch::SketchSolution>,
) -> Vec<Value> {
    sketch
        .constraints()
        .map(|(id, constraint)| {
            let mut entry = Map::new();
            entry.insert("id".to_owned(), json!(id.raw()));
            entry.insert(
                "description".to_owned(),
                json!(sketch.describe_constraint(id)),
            );
            if let Some(expression) = constraint.dimension() {
                entry.insert(
                    "expression".to_owned(),
                    json!(document.expression_text(expression)),
                );
                if let Some(value) = solution.and_then(|solution| solution.dimension(id)) {
                    entry.insert("value".to_owned(), rounded(value));
                }
            }
            if !sketch.is_active(id) {
                entry.insert("active".to_owned(), json!(false));
            }
            Value::Object(entry)
        })
        .collect()
}

pub fn render(
    session: &mut Session,
    work: &Work<'_>,
    picture: &Picture,
) -> Result<Answer, ToolError> {
    let size = picture_size(picture.width, picture.height)?;
    session.evaluate(work)?;
    let document = session.document();
    let evaluation = session.evaluation();
    let home = snapshot::home_viewpoint(document);
    let starting = match picture.view.unwrap_or(ViewName::Home) {
        ViewName::Home => home,
        other => Viewpoint::looking_from(
            standard_view(other).looking_from(),
            Point3::ZERO,
            home.distance,
        )
        .unwrap_or(home),
    };
    work.progress("Preparing the bodies for drawing");
    let cancel = work.cancel_token();
    let snapshot = snapshot::looking_from(document, evaluation, size, starting, &cancel).map_err(
        |unmeshed| match unmeshed {
            Unmeshed::Cancelled => ToolError::Cancelled(Cancelled),
            Unmeshed::TimedOut => RenderError::Unmeshed(unmeshed).into(),
        },
    )?;
    work.progress("Drawing the picture");
    let mut renderer =
        OffscreenRenderer::new(GraphicsSettings::default()).map_err(RenderError::NoAdapter)?;
    let bands = renderer
        .render(&ImageRequest {
            size,
            view: &snapshot.view,
            scene: &snapshot.scene,
            pixels_per_point: snapshot.pixels_per_point,
            background: Background::Viewport,
        })
        .map_err(RenderError::Drawing)?;
    let png = png_bytes(&mut RenderedRows(bands), &cancel).map_err(RenderError::Encoding)?;
    if work.is_cancelled() {
        return Err(Cancelled.into());
    }
    let mut data = Map::new();
    data.insert("width".to_owned(), json!(size.width));
    data.insert("height".to_owned(), json!(size.height));
    data.insert("bodies".to_owned(), json!(evaluation.bodies().count()));
    Ok(Answer {
        data,
        image: Some(png),
    })
}

pub fn picture_size(width: Option<u32>, height: Option<u32>) -> Result<SurfaceSize, RenderError> {
    let side = |side: Option<u32>, default: u32| {
        let side = side.unwrap_or(default);
        if (SMALLEST_SIDE..=LARGEST_SIDE).contains(&side) {
            Ok(side)
        } else {
            Err(RenderError::Size(side))
        }
    };
    Ok(SurfaceSize {
        width: side(width, DEFAULT_SIZE.width)?,
        height: side(height, DEFAULT_SIZE.height)?,
    })
}

fn standard_view(view: ViewName) -> StandardView {
    match view {
        ViewName::Home | ViewName::Isometric => StandardView::Isometric,
        ViewName::Front => StandardView::Front,
        ViewName::Top => StandardView::Top,
        ViewName::Right => StandardView::Right,
        ViewName::Back => StandardView::Back,
        ViewName::Bottom => StandardView::Bottom,
        ViewName::Left => StandardView::Left,
    }
}
