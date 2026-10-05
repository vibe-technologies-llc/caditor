use std::{
    collections::{BTreeMap, BTreeSet},
    panic::{self, AssertUnwindSafe},
    sync::OnceLock,
};

use caditor_expression::{Dimension, EvalError, Expression, ParameterId, Quantity};
use caditor_geometry::{Aabb, Aabb2, Plane, Point2, Vector2};
use caditor_kernel::{
    AngularExtent, Axis2, BooleanError, BooleanOperation, EdgeId, EdgeName, FaceId, FaceName,
    GeometryError, LINEAR_RESOLUTION, LinearBound, LinearExtent, MAX_SIZE, Mesh, MeshQuality,
    OpenEnd, Profile, ProfileCurve, ProfileError, ReachError, Region, RegionMesh, RegionReference,
    SamplingTolerance, Selection, Solid, SweepError, TessellationError, VertexId, VertexName,
    boolean, extrude, heights, next_face, resolve_regions, revolve, vertex_names,
};
use caditor_sketch::{Entity, EntityId, Reference, Sketch};

use crate::{
    attachment::AttachmentError,
    datum::{AxisReference, PlaneReference, Resolver, capitalized, describe_axis, describe_plane},
    describe::describe_origin,
    document::{Feature, FeatureId, list_names},
    recompute::{
        CancelToken, Failure, FeatureError, FeatureResult, FixTarget, Inputs, SketchResult,
    },
    tolerance,
    trouble::{self, boolean_trouble},
    values::ParameterValues,
};

const THROUGH_ALL_MARGIN: f64 = 1.0;
const THROUGH_ALL_REACH: f64 = 0.05;

#[derive(Debug, Clone, PartialEq, Default)]
pub enum RegionChoice {
    #[default]
    All,
    Chosen(Vec<RegionReference>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BodyOperation {
    NewBody,
    Add(FeatureId),
    Remove(FeatureId),
    Intersect(FeatureId),
}

impl BodyOperation {
    pub fn target(self) -> Option<FeatureId> {
        match self {
            Self::NewBody => None,
            Self::Add(body) | Self::Remove(body) | Self::Intersect(body) => Some(body),
        }
    }

    #[must_use]
    pub fn with_target(self, body: FeatureId) -> Self {
        match self {
            Self::NewBody => Self::NewBody,
            Self::Add(_) => Self::Add(body),
            Self::Remove(_) => Self::Remove(body),
            Self::Intersect(_) => Self::Intersect(body),
        }
    }

    pub fn verb(self) -> &'static str {
        match self {
            Self::NewBody => "New body",
            Self::Add(_) => "Add",
            Self::Remove(_) => "Remove",
            Self::Intersect(_) => "Intersect",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExtrudeEnd {
    Distance(Expression),
    ThroughAll,
    UpToNext,
    UpToFace(PlaneReference),
}

impl ExtrudeEnd {
    pub fn distance(&self) -> Option<&Expression> {
        match self {
            Self::Distance(distance) => Some(distance),
            Self::ThroughAll | Self::UpToNext | Self::UpToFace(_) => None,
        }
    }

    pub fn target(&self) -> Option<&PlaneReference> {
        match self {
            Self::UpToFace(target) => Some(target),
            Self::Distance(_) | Self::ThroughAll | Self::UpToNext => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExtrudeExtent {
    OneSide {
        end: ExtrudeEnd,
        reversed: bool,
    },
    Symmetric {
        distance: Expression,
    },
    TwoSides {
        forward: ExtrudeEnd,
        backward: ExtrudeEnd,
    },
}

impl ExtrudeExtent {
    pub fn one_side(distance: Expression, reversed: bool) -> Self {
        Self::OneSide {
            end: ExtrudeEnd::Distance(distance),
            reversed,
        }
    }

    pub fn two_sides(forward: Expression, backward: Expression) -> Self {
        Self::TwoSides {
            forward: ExtrudeEnd::Distance(forward),
            backward: ExtrudeEnd::Distance(backward),
        }
    }

    pub fn ends(&self) -> Vec<&ExtrudeEnd> {
        match self {
            Self::OneSide { end, .. } => vec![end],
            Self::Symmetric { .. } => Vec::new(),
            Self::TwoSides { forward, backward } => vec![forward, backward],
        }
    }

    fn expressions(&self) -> Vec<&Expression> {
        match self {
            Self::Symmetric { distance } => vec![distance],
            Self::OneSide { .. } | Self::TwoSides { .. } => self
                .ends()
                .into_iter()
                .filter_map(ExtrudeEnd::distance)
                .collect(),
        }
    }

    fn targets(&self) -> Vec<&PlaneReference> {
        self.ends()
            .into_iter()
            .filter_map(ExtrudeEnd::target)
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum RevolveExtent {
    Full,
    OneSide {
        angle: Expression,
        reversed: bool,
    },
    Symmetric {
        angle: Expression,
    },
    TwoSides {
        forward: Expression,
        backward: Expression,
    },
}

impl RevolveExtent {
    fn expressions(&self) -> Vec<&Expression> {
        match self {
            Self::Full => Vec::new(),
            Self::OneSide { angle, .. } | Self::Symmetric { angle } => vec![angle],
            Self::TwoSides { forward, backward } => vec![forward, backward],
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum SolidStart {
    Distance(Expression),
    Plane(PlaneReference),
}

impl SolidStart {
    pub fn distance(&self) -> Option<&Expression> {
        match self {
            Self::Distance(distance) => Some(distance),
            Self::Plane(_) => None,
        }
    }

    pub fn target(&self) -> Option<&PlaneReference> {
        match self {
            Self::Plane(target) => Some(target),
            Self::Distance(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Extrude {
    pub sketch: FeatureId,
    pub regions: RegionChoice,
    pub extent: ExtrudeExtent,
    pub operation: BodyOperation,
    pub start: Option<SolidStart>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RevolveAxis {
    Sketch(EntityId),
    Model(AxisReference),
}

impl RevolveAxis {
    pub fn line(&self) -> Option<EntityId> {
        match self {
            Self::Sketch(line) => Some(*line),
            Self::Model(_) => None,
        }
    }

    pub fn model(&self) -> Option<&AxisReference> {
        match self {
            Self::Model(axis) => Some(axis),
            Self::Sketch(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Revolve {
    pub sketch: FeatureId,
    pub regions: RegionChoice,
    pub axis: RevolveAxis,
    pub extent: RevolveExtent,
    pub operation: BodyOperation,
    pub start: Option<SolidStart>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SolidFeature {
    Extrude(Extrude),
    Revolve(Revolve),
}

impl SolidFeature {
    pub fn sketch(&self) -> FeatureId {
        match self {
            Self::Extrude(extrude) => extrude.sketch,
            Self::Revolve(revolve) => revolve.sketch,
        }
    }

    pub fn regions(&self) -> &RegionChoice {
        match self {
            Self::Extrude(extrude) => &extrude.regions,
            Self::Revolve(revolve) => &revolve.regions,
        }
    }

    pub fn operation(&self) -> BodyOperation {
        match self {
            Self::Extrude(extrude) => extrude.operation,
            Self::Revolve(revolve) => revolve.operation,
        }
    }

    pub fn axis(&self) -> Option<&RevolveAxis> {
        match self {
            Self::Extrude(_) => None,
            Self::Revolve(revolve) => Some(&revolve.axis),
        }
    }

    pub fn axis_line(&self) -> Option<EntityId> {
        self.axis().and_then(RevolveAxis::line)
    }

    pub fn axis_body(&self) -> Option<FeatureId> {
        self.axis()
            .and_then(RevolveAxis::model)
            .and_then(AxisReference::body)
    }

    pub fn axis_datum(&self) -> Option<FeatureId> {
        self.axis()
            .and_then(RevolveAxis::model)
            .and_then(AxisReference::datum)
    }

    pub fn start(&self) -> Option<&SolidStart> {
        match self {
            Self::Extrude(extrude) => extrude.start.as_ref(),
            Self::Revolve(revolve) => revolve.start.as_ref(),
        }
    }

    fn targets(&self) -> Vec<&PlaneReference> {
        let ends = match self {
            Self::Extrude(extrude) => extrude.extent.targets(),
            Self::Revolve(_) => Vec::new(),
        };
        ends.into_iter()
            .chain(self.start().and_then(SolidStart::target))
            .collect()
    }

    pub fn end_bodies(&self) -> BTreeSet<FeatureId> {
        self.targets()
            .into_iter()
            .filter_map(PlaneReference::body)
            .collect()
    }

    pub fn origin_features(&self) -> BTreeSet<FeatureId> {
        self.targets()
            .into_iter()
            .flat_map(PlaneReference::origin_features)
            .chain(
                self.axis()
                    .and_then(RevolveAxis::model)
                    .into_iter()
                    .flat_map(AxisReference::origin_features),
            )
            .collect()
    }

    pub fn end_datums(&self) -> BTreeSet<FeatureId> {
        self.targets()
            .into_iter()
            .filter_map(PlaneReference::datum)
            .collect()
    }

    pub fn shape(&self) -> &'static str {
        match self {
            Self::Extrude(_) => "extrusion",
            Self::Revolve(_) => "revolution",
        }
    }

    pub fn same_kind(&self, other: &Self) -> bool {
        matches!(
            (self, other),
            (Self::Extrude(_), Self::Extrude(_)) | (Self::Revolve(_), Self::Revolve(_))
        )
    }

    fn expressions(&self) -> Vec<&Expression> {
        match self {
            Self::Extrude(extrude) => {
                let mut expressions = extrude.extent.expressions();
                expressions.extend(self.start().and_then(SolidStart::distance));
                expressions
            }
            Self::Revolve(revolve) => {
                let mut expressions = revolve.extent.expressions();
                expressions.extend(self.start().and_then(SolidStart::distance));
                expressions
            }
        }
    }

    pub fn heap_size(&self) -> usize {
        let chosen = match self.regions() {
            RegionChoice::All => 0,
            RegionChoice::Chosen(regions) => {
                size_of_val(regions.as_slice())
                    + regions
                        .iter()
                        .map(RegionReference::heap_size)
                        .sum::<usize>()
            }
        };
        let expressions: usize = self
            .expressions()
            .into_iter()
            .map(Expression::heap_size)
            .sum();
        let planes: usize = self
            .targets()
            .into_iter()
            .map(PlaneReference::heap_size)
            .sum();
        let axis = match self {
            Self::Extrude(_) => 0,
            Self::Revolve(revolve) => match &revolve.axis {
                RevolveAxis::Model(axis) => axis.heap_size(),
                RevolveAxis::Sketch(_) => 0,
            },
        };
        let references = planes + axis;
        chosen + expressions + references
    }

    pub fn parameters(&self) -> BTreeSet<ParameterId> {
        self.expressions()
            .into_iter()
            .flat_map(Expression::parameters)
            .collect()
    }

    pub fn uses_parameter(&self, parameter: ParameterId) -> bool {
        self.expressions()
            .into_iter()
            .any(|expression| expression.uses(parameter))
    }

    pub fn features(&self) -> BTreeSet<FeatureId> {
        std::iter::once(self.sketch())
            .chain(self.operation().target())
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SolidResult {
    pub body: FeatureId,
    pub solid: Solid,
    mesh: OnceLock<Option<Mesh>>,
    bounds: OnceLock<Option<Aabb>>,
    names: OnceLock<NameIndex>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct NameIndex {
    faces: BTreeMap<FaceName, Vec<FaceId>>,
    edges: BTreeMap<EdgeName, EdgeId>,
    vertices: BTreeMap<VertexName, Vec<VertexId>>,
    vertex_names: BTreeMap<VertexId, VertexName>,
}

impl NameIndex {
    pub fn of(solid: &Solid) -> Self {
        let mut faces: BTreeMap<FaceName, Vec<FaceId>> = BTreeMap::new();
        for (id, face) in solid.faces() {
            faces.entry(face.name()).or_default().push(id);
        }
        let mut edges = BTreeMap::new();
        for (id, edge) in solid.edges() {
            edges.entry(edge.name()).or_insert(id);
        }
        let vertex_names = vertex_names(solid);
        let mut vertices: BTreeMap<VertexName, Vec<VertexId>> = BTreeMap::new();
        for (id, name) in &vertex_names {
            vertices.entry(*name).or_default().push(*id);
        }
        Self {
            faces,
            edges,
            vertices,
            vertex_names,
        }
    }

    pub fn vertices_named(&self, name: VertexName) -> &[VertexId] {
        self.vertices.get(&name).map_or(&[], Vec::as_slice)
    }

    pub fn vertex_name(&self, vertex: VertexId) -> Option<VertexName> {
        self.vertex_names.get(&vertex).copied()
    }

    pub fn faces_named(&self, name: FaceName) -> &[FaceId] {
        self.faces.get(&name).map_or(&[], Vec::as_slice)
    }

    pub fn edge_named(&self, name: EdgeName) -> Option<EdgeId> {
        self.edges.get(&name).copied()
    }
}

impl SolidResult {
    pub fn new(body: FeatureId, solid: Solid) -> Self {
        Self {
            body,
            solid,
            mesh: OnceLock::new(),
            bounds: OnceLock::new(),
            names: OnceLock::new(),
        }
    }

    pub fn names(&self) -> &NameIndex {
        self.names.get_or_init(|| NameIndex::of(&self.solid))
    }

    pub fn bounding_box(&self) -> Option<Aabb> {
        *self.bounds.get_or_init(|| self.solid.bounding_box())
    }

    pub fn mesh(&self) -> Option<&Mesh> {
        self.mesh.get().and_then(Option::as_ref)
    }

    pub fn is_meshed(&self) -> bool {
        self.mesh.get().is_some()
    }

    pub fn mesh_failed(&self) -> bool {
        matches!(self.mesh.get(), Some(None))
    }

    pub(crate) fn tessellate(&self, name: &str, quality: &MeshQuality) {
        if self.is_meshed() {
            return;
        }
        let tessellated = panic::catch_unwind(AssertUnwindSafe(|| {
            self.bounding_box();
            self.solid.display_mesh(quality)
        }));
        let mesh = match tessellated {
            Ok(Ok(mesh)) => Some(mesh),
            Ok(Err(TessellationError::Cancelled(_))) => return,
            Ok(Err(error)) => {
                log::warn!("the body of {name} could not be meshed: {error}");
                None
            }
            Err(_) => {
                log::error!("meshing the body of {name} panicked");
                None
            }
        };
        let _ = self.mesh.set(mesh);
    }
}

pub(crate) fn profile_curves(sketch: &Sketch) -> Vec<ProfileCurve> {
    sketch
        .entities()
        .filter(|(id, _)| !sketch.is_construction(*id))
        .filter_map(|(id, entity)| {
            let raw = id.raw();
            match entity {
                Entity::Point(_) => None,
                Entity::Line { .. } => sketch
                    .line_endpoints(id)
                    .map(|(start, end)| ProfileCurve::line(raw, start, end)),
                Entity::Circle { .. } => sketch
                    .circle(id)
                    .map(|(center, radius)| ProfileCurve::circle(raw, center, radius)),
                Entity::Arc { center, start, end } => Some(ProfileCurve::arc(
                    raw,
                    sketch.point(*center)?,
                    sketch.point(*start)?,
                    sketch.point(*end)?,
                )),
                Entity::Spline { .. } => sketch.spline(id).map(|spline| {
                    ProfileCurve::spline(
                        raw,
                        spline.degree(),
                        spline.knots().to_vec(),
                        spline.control_points().to_vec(),
                    )
                }),
            }
        })
        .collect()
}

pub fn sketch_regions(sketch: &Sketch) -> Result<Vec<Region>, ProfileError> {
    Ok(Profile::new(&profile_curves(sketch))?.regions().to_vec())
}

#[derive(Debug, Clone, PartialEq)]
pub struct SketchRegion {
    pub region: Region,
    pub mesh: Option<RegionMesh>,
    pub even_depth: bool,
}

impl SketchRegion {
    pub fn reference(&self) -> RegionReference {
        let anchor = self
            .mesh
            .as_ref()
            .and_then(RegionMesh::anchor)
            .or_else(|| self.region.anchor());
        RegionReference::capture(&self.region, anchor)
    }
}

pub(crate) fn display_regions(profile: &Profile) -> Result<Vec<SketchRegion>, ProfileError> {
    let extent = profile
        .regions()
        .iter()
        .filter_map(Region::bounds)
        .reduce(Aabb2::union)
        .map_or(1.0, |bounds| bounds.size().length());
    let tolerance = SamplingTolerance::for_extent(extent);
    Ok(profile
        .regions()
        .iter()
        .map(|region| SketchRegion {
            mesh: region.triangulate(&tolerance),
            even_depth: region.depth() % 2 == 0,
            region: region.clone(),
        })
        .collect())
}

struct Context<'a> {
    feature: &'a Feature,
    sketch_name: String,
    sketch_id: FeatureId,
    sketch: &'a Sketch,
}

impl Context<'_> {
    fn error(&self, reason: String, remedy: String, fix: FixTarget) -> Failure {
        Failure::Error(Box::new(FeatureError {
            reason,
            remedy,
            fix: Some(fix),
            constraints: Vec::new(),
            place: None,
        }))
    }

    fn own(&self) -> FixTarget {
        FixTarget::Feature(self.feature.id())
    }

    fn in_sketch(&self) -> FixTarget {
        FixTarget::Feature(self.sketch_id)
    }

    fn curves(&self, entities: &[u64]) -> String {
        let labels: Vec<String> = entities
            .iter()
            .map(|raw| self.sketch.entity_label(EntityId::from_raw(*raw)))
            .collect();
        crate::document::list_names(&labels)
    }
}

pub(crate) fn evaluate(
    feature: &Feature,
    solid: &SolidFeature,
    inputs: &Inputs<'_>,
    cancel: &CancelToken,
) -> Result<FeatureResult, Failure> {
    let sketch_id = solid.sketch();
    let sketch_name = inputs
        .document
        .feature(sketch_id)
        .map(|sketch| sketch.name.clone())
        .unwrap_or_default();
    let Some(FeatureResult::Sketch(sketch)) = inputs.features.get(&sketch_id).map(AsRef::as_ref)
    else {
        return Err(Failure::Error(Box::new(FeatureError {
            reason: format!("It needs the shape of {sketch_name}, which is not available."),
            remedy: format!("Fix {sketch_name} first."),
            fix: Some(FixTarget::Feature(sketch_id)),
            constraints: Vec::new(),
            place: None,
        })));
    };
    let context = Context {
        feature,
        sketch_name,
        sketch_id,
        sketch: &sketch.geometry,
    };
    let regions = chosen_regions(&context, sketch, solid.regions())?;
    if cancel.is_cancelled() {
        return Err(Failure::Cancelled);
    }
    let plane = sketch.geometry.plane();
    let raw = feature.id().raw();
    let started = match solid.start() {
        Some(start) => start_plane(&context, inputs, solid.shape(), plane, start)?,
        None => plane,
    };
    let tool = match solid {
        SolidFeature::Extrude(definition) => {
            let plane = started;
            let ends = Ends {
                context: &context,
                inputs,
                operation: definition.operation,
                plane,
                regions: &regions,
            };
            let extent = ends.extent(&definition.extent)?;
            extrude(&plane, &regions, extent, raw)
        }
        SolidFeature::Revolve(definition) => {
            let axis = match &definition.axis {
                RevolveAxis::Sketch(line) => revolution_axis(&context, *line)?,
                RevolveAxis::Model(reference) => {
                    model_axis(&context, feature, inputs, &plane, reference)?
                }
            };
            let extent = angular_extent(&context, &definition.extent, inputs.parameters)?;
            revolve(&started, &regions, axis, extent, raw)
        }
    }
    .map_err(|error| sweep_failure(&context, solid.shape(), &error))?;
    if cancel.is_cancelled() {
        return Err(Failure::Cancelled);
    }
    let (body, solid) = match solid.operation() {
        BodyOperation::NewBody => (feature.id(), tool),
        operation @ (BodyOperation::Add(body)
        | BodyOperation::Remove(body)
        | BodyOperation::Intersect(body)) => {
            let current = inputs
                .body(body)
                .ok_or_else(|| missing_body(inputs, body))?;
            let kernel_operation = match operation {
                BodyOperation::Remove(_) => BooleanOperation::Difference,
                BodyOperation::Intersect(_) => BooleanOperation::Intersection,
                BodyOperation::NewBody | BodyOperation::Add(_) => BooleanOperation::Union,
            };
            let combined = boolean(current, &tool, kernel_operation).map_err(|error| {
                boolean_failure(&context, inputs, [current, &tool], body, operation, &error)
            })?;
            (body, combined)
        }
    };
    Ok(FeatureResult::Solid(SolidResult::new(body, solid)))
}

fn resolve_target(
    context: &Context<'_>,
    inputs: &Inputs<'_>,
    target: &PlaneReference,
    subject: &str,
    remedy: &str,
) -> Result<Plane, Failure> {
    let PlaneReference::Face(attachment) = target else {
        return Resolver {
            feature: context.feature,
            inputs,
        }
        .plane(target);
    };
    let body = feature_name(inputs, attachment.body);
    let solid = inputs
        .body(attachment.body)
        .ok_or_else(|| missing_body(inputs, attachment.body))?;
    attachment.resolve(solid).map_err(|error| {
        let reason = match error {
            AttachmentError::Missing => {
                format!("The face {subject} is no longer part of the body of {body}.")
            }
            AttachmentError::Ambiguous => format!(
                "The face of {body} {subject} was split into parts that no longer lie in one \
                 plane."
            ),
            AttachmentError::NotFlat => {
                format!("The face of {body} {subject} is no longer flat.")
            }
        };
        context.error(reason, remedy.to_owned(), context.own())
    })
}

fn start_plane(
    context: &Context<'_>,
    inputs: &Inputs<'_>,
    shape: &str,
    plane: Plane,
    start: &SolidStart,
) -> Result<Plane, Failure> {
    match start {
        SolidStart::Distance(offset) => offset_plane(context, plane, offset, inputs.parameters),
        SolidStart::Plane(target) => {
            let subject = format!("this {shape} starts from");
            let remedy = "Select a flat face or plane and use it as the start, or enter a \
                          distance.";
            let found = resolve_target(context, inputs, target, &subject, remedy)?;
            if !tolerance::parallel(found.normal(), plane.normal()) {
                return Err(context.error(
                    format!(
                        "{} is not parallel to the plane of {}, so the {shape} cannot start \
                         from it.",
                        capitalized(&describe_plane(inputs.document, target)),
                        context.sketch_name
                    ),
                    "Choose a face or plane parallel to the sketch, or enter a distance."
                        .to_owned(),
                    context.own(),
                ));
            }
            let distance = plane.signed_distance(found.origin());
            within_reach(context, distance.abs(), "start offset")?;
            Plane::from_frame(
                plane.origin() + plane.normal() * distance,
                plane.normal(),
                plane.x_axis(),
            )
            .ok_or_else(|| {
                context.error(
                    format!(
                        "The {shape} cannot start from the plane of {}.",
                        context.sketch_name
                    ),
                    "Choose another face or plane.".to_owned(),
                    context.own(),
                )
            })
        }
    }
}

fn offset_plane(
    context: &Context<'_>,
    plane: Plane,
    offset: &Expression,
    parameters: &ParameterValues,
) -> Result<Plane, Failure> {
    const WHAT: &str = "start offset";
    let distance = evaluate_value(context, offset, Dimension::LENGTH, WHAT, parameters)?;
    within_reach(context, distance.abs(), WHAT)?;
    Plane::from_frame(
        plane.origin() + plane.normal() * distance,
        plane.normal(),
        plane.x_axis(),
    )
    .ok_or_else(|| {
        context.error(
            format!(
                "The {WHAT} cannot be applied to the plane of {}.",
                context.sketch_name
            ),
            "Enter another start offset.".to_owned(),
            context.own(),
        )
    })
}

fn missing_body(inputs: &Inputs<'_>, body: FeatureId) -> Failure {
    let name = inputs
        .document
        .feature(body)
        .map(|feature| feature.name.clone())
        .unwrap_or_default();
    Failure::Error(Box::new(FeatureError {
        reason: format!("The body made by {name} has no shape."),
        remedy: format!("Fix {name} first."),
        fix: Some(FixTarget::Feature(body)),
        constraints: Vec::new(),
        place: None,
    }))
}

fn chosen_regions(
    context: &Context<'_>,
    sketch: &SketchResult,
    choice: &RegionChoice,
) -> Result<Vec<Region>, Failure> {
    let profile = sketch
        .profile()
        .map_err(|error| profile_failure(context, &error))?;
    let selection = match choice {
        RegionChoice::All => Selection::EvenDepth,
        RegionChoice::Chosen(references) => Selection::Regions(
            resolve_regions(references, profile.regions())
                .map_err(|error| profile_failure(context, &error))?
                .keys,
        ),
    };
    profile
        .select(&selection)
        .map_err(|error| profile_failure(context, &error))
}

const MAX_NAMED_GAPS: usize = 2;

fn open_gaps(context: &Context<'_>, open_ends: &[OpenEnd]) -> Option<String> {
    let mut gaps: Vec<(u64, u64, f64)> = open_ends
        .iter()
        .filter_map(|end| {
            let near = end.nearest?;
            Some((
                end.entity.min(near.entity),
                end.entity.max(near.entity),
                near.gap,
            ))
        })
        .collect();
    gaps.sort_by(|a, b| a.2.total_cmp(&b.2).then(a.0.cmp(&b.0)).then(a.1.cmp(&b.1)));
    gaps.dedup_by(|a, b| (a.0, a.1) == (b.0, b.1));
    let named: Vec<String> = gaps
        .iter()
        .take(MAX_NAMED_GAPS)
        .map(|(first, second, gap)| {
            format!(
                "an end of {} is {} from an end of {}",
                context.curves(&[*first]),
                Quantity::length(*gap),
                context.curves(&[*second])
            )
        })
        .collect();
    named.first()?;
    let unnamed = gaps.len().saturating_sub(MAX_NAMED_GAPS);
    let mut text = named.join(" and ");
    if unnamed > 0 {
        let noun = if unnamed == 1 { "gap" } else { "gaps" };
        text.push_str(&format!(", plus {unnamed} more {noun}"));
    }
    Some(text)
}

fn profile_failure(context: &Context<'_>, error: &ProfileError) -> Failure {
    let sketch = &context.sketch_name;
    match error {
        ProfileError::Cancelled(_) => Failure::Cancelled,
        ProfileError::NoClosedProfile { open_ends } => {
            let reason = match open_gaps(context, open_ends) {
                Some(gaps) => format!("{sketch} has no closed shape to sweep: {gaps}."),
                None => format!("{sketch} has no closed shape to sweep."),
            };
            context.error(
                reason,
                format!(
                    "Close the outline in {sketch}, for example by joining the ends of its lines."
                ),
                context.in_sketch(),
            )
        }
        ProfileError::EmptySelection => context.error(
            "No region of the sketch is chosen.".to_owned(),
            "Choose at least one region.".to_owned(),
            context.own(),
        ),
        ProfileError::MissingRegion(_) => context.error(
            format!("A chosen region of {sketch} no longer exists, because the sketch changed."),
            "Choose the regions again.".to_owned(),
            context.own(),
        ),
        ProfileError::AmbiguousRegion { candidates, .. } => context.error(
            format!(
                "A chosen region of {sketch} was divided, and {} of the new regions match it equally.",
                candidates.len()
            ),
            "Choose the regions again.".to_owned(),
            context.own(),
        ),
        ProfileError::Unresolved { entities } if entities.is_empty() => context.error(
            format!("The curves of {sketch} could not be divided into closed regions."),
            format!("Simplify where curves meet or cross in {sketch}."),
            context.in_sketch(),
        ),
        ProfileError::DuplicateEntity { .. }
        | ProfileError::Degenerate { .. }
        | ProfileError::InvalidCurve { .. }
        | ProfileError::SelfOverlap { .. }
        | ProfileError::Overlap { .. }
        | ProfileError::TooIntricate { .. }
        | ProfileError::Unresolved { .. } => {
            let entities = error.entities();
            let curves = context.curves(&entities);
            let problem = match error {
                ProfileError::Degenerate { .. } => "has no length",
                ProfileError::InvalidCurve {
                    error: GeometryError::BeyondMaximum(_),
                    ..
                } => "reaches farther than caditor models",
                ProfileError::SelfOverlap { .. } => "runs back over itself",
                ProfileError::Overlap { .. } => "run along each other",
                ProfileError::TooIntricate { .. } => "cross too often to be divided into regions",
                ProfileError::Unresolved { .. } => "could not be divided into closed regions",
                _ => "cannot be used",
            };
            let which = if entities.len() > 1 {
                "one of them"
            } else {
                "it"
            };
            context.error(
                format!("In {sketch}, {curves} {problem}."),
                format!("Change or delete {which} in {sketch}."),
                context.in_sketch(),
            )
        }
    }
}

fn sweep_failure(context: &Context<'_>, shape: &str, error: &SweepError) -> Failure {
    let sketch = &context.sketch_name;
    match error {
        SweepError::Cancelled(_) => Failure::Cancelled,
        SweepError::NoRegions => context.error(
            "No region of the sketch is chosen.".to_owned(),
            "Choose at least one region.".to_owned(),
            context.own(),
        ),
        SweepError::NonFinite | SweepError::ZeroLength => context.error(
            format!("The {shape} has no length."),
            "Enter a distance other than zero.".to_owned(),
            context.own(),
        ),
        SweepError::TooLong => context.error(
            format!("The {shape} reaches farther than {} m.", MAX_SIZE / 1_000.0),
            "Enter a shorter distance.".to_owned(),
            context.own(),
        ),
        SweepError::ZeroAngle => context.error(
            "The revolution has no angle.".to_owned(),
            "Enter an angle other than zero.".to_owned(),
            context.own(),
        ),
        SweepError::BeyondFullTurn => context.error(
            "The revolution turns more than once.".to_owned(),
            "Enter an angle of at most 360°.".to_owned(),
            context.own(),
        ),
        SweepError::EndAlongDirection => context.error(
            "An end of the extrusion runs along its direction, so the extrusion never reaches it."
                .to_owned(),
            "Choose a face or plane that faces the sketch, or enter a distance.".to_owned(),
            context.own(),
        ),
        SweepError::EndsCross => context.error(
            format!("The two ends of the extrusion meet or cross within the profile of {sketch}."),
            "Choose ends that stay apart across the whole profile.".to_owned(),
            context.own(),
        ),
        SweepError::DegenerateAxis => context.error(
            "The revolution axis has no direction.".to_owned(),
            "Choose a line as the axis.".to_owned(),
            context.own(),
        ),
        SweepError::BothSidesOfAxis { left, right } => {
            let apart = if right.len() <= left.len() {
                right
            } else {
                left
            };
            let verb = if apart.len() == 1 { "lies" } else { "lie" };
            let curves = context.curves(apart);
            context.error(
                format!(
                    "In {sketch}, {curves} {verb} on the other side of the revolution axis from the \
                     rest of the profile."
                ),
                "Choose only the regions on one side of the axis, or choose another axis."
                    .to_owned(),
                context.own(),
            )
        }
        SweepError::CrossesAxis { .. } => {
            let curves = context.curves(&error.entities());
            context.error(
                format!("In {sketch}, {curves} would cut through the revolution axis."),
                format!(
                    "Keep the profile on one side of the axis in {sketch}, or choose another axis."
                ),
                context.in_sketch(),
            )
        }
        SweepError::OnAxis => context.error(
            format!("The profile of {sketch} lies on the revolution axis."),
            "Choose another axis.".to_owned(),
            context.own(),
        ),
        SweepError::Unassembled | SweepError::Geometry(_) | SweepError::Invalid(_) => {
            log::warn!("{} could not be built: {error}", context.feature.name);
            context.error(
                format!("The {shape} of {sketch} could not be built."),
                format!("Undo the last change, or simplify the regions of {sketch}."),
                context.own(),
            )
        }
    }
}

fn boolean_failure(
    context: &Context<'_>,
    inputs: &Inputs<'_>,
    operands: [&Solid; 2],
    body: FeatureId,
    operation: BodyOperation,
    error: &BooleanError,
) -> Failure {
    let body_name = inputs
        .document
        .feature(body)
        .map(|feature| feature.name.clone())
        .unwrap_or_default();
    let failure = match error {
        BooleanError::Cancelled(_) => Failure::Cancelled,
        BooleanError::Empty => {
            let reason = match operation {
                BodyOperation::Intersect(_) => {
                    format!(
                        "The shape does not overlap the body of {body_name}, so nothing would be left."
                    )
                }
                _ => format!("This removes all of the body of {body_name}."),
            };
            context.error(
                reason,
                "Change the distance or the sketch so that part of the body stays.".to_owned(),
                context.own(),
            )
        }
        BooleanError::NonManifold(_) => context.error(
            format!(
                "The body of {body_name} would be left with parts that meet only along an edge."
            ),
            "Move or resize the shape so it overlaps more or stays clear.".to_owned(),
            context.own(),
        ),
        BooleanError::Intersection { .. }
        | BooleanError::Split(_)
        | BooleanError::Ambiguous(_)
        | BooleanError::Open(_)
        | BooleanError::Invalid(_) => {
            log::warn!("{} could not be combined: {error}", context.feature.name);
            let trouble =
                boolean_trouble(inputs.document, operands, error, "Move or resize the shape");
            context.error(
                trouble.reason(format!(
                    "The shape could not be combined with the body of {body_name}."
                )),
                trouble.remedy,
                context.own(),
            )
        }
    };
    failure.placed(trouble::place(error))
}

fn evaluate_value(
    context: &Context<'_>,
    expression: &Expression,
    dimension: Dimension,
    what: &str,
    parameters: &ParameterValues,
) -> Result<f64, Failure> {
    expression
        .evaluate_as(dimension, &|id| parameters.value(id))
        .map_err(|error| {
            let (remedy, fix) = match &error {
                EvalError::ParameterFailed { id, name } => (
                    format!("Fix {name} under Parameters, or edit the {what}."),
                    FixTarget::Parameter(*id),
                ),
                EvalError::WrongKind { .. } if dimension == Dimension::ANGLE => (
                    format!("Edit the {what} so it gives an angle, such as 90 deg."),
                    context.own(),
                ),
                EvalError::WrongKind { .. } => (
                    format!("Edit the {what} so it gives a length, such as 10 mm."),
                    context.own(),
                ),
                _ => (
                    format!("Edit the {what} or the parameters it uses."),
                    context.own(),
                ),
            };
            context.error(
                format!("The {what} cannot be evaluated: {error}."),
                remedy,
                fix,
            )
        })
}

fn positive(context: &Context<'_>, value: f64, what: &str) -> Result<f64, Failure> {
    if value > 0.0 {
        Ok(value)
    } else {
        Err(context.error(
            format!("The {what} must be more than zero."),
            format!("Enter a {what} above zero, or flip the direction instead."),
            context.own(),
        ))
    }
}

fn within_reach(context: &Context<'_>, value: f64, what: &str) -> Result<f64, Failure> {
    if value <= MAX_SIZE {
        return Ok(value);
    }
    let most = MAX_SIZE / 1_000.0;
    Err(context.error(
        format!("The {what} cannot be more than {most} m."),
        format!("Enter a {what} of at most {most} m."),
        context.own(),
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    Only { reversed: bool },
    Forward,
    Backward,
}

impl Side {
    fn reversed(self) -> bool {
        match self {
            Self::Only { reversed } => reversed,
            Self::Forward => false,
            Self::Backward => true,
        }
    }

    fn sign(self) -> f64 {
        if self.reversed() { -1.0 } else { 1.0 }
    }

    fn distance(self) -> &'static str {
        match self {
            Self::Only { .. } => "distance",
            Self::Forward => "forward distance",
            Self::Backward => "backward distance",
        }
    }

    fn ahead(self) -> &'static str {
        match self {
            Self::Only { .. } => "ahead of the sketch",
            Self::Forward => "on the forward side of the sketch",
            Self::Backward => "on the backward side of the sketch",
        }
    }

    fn other_way(self) -> &'static str {
        match self {
            Self::Only { .. } => "Turn Reversed on or off to go the other way",
            Self::Forward | Self::Backward => "Choose another end for that side",
        }
    }
}

fn feature_name(inputs: &Inputs<'_>, feature: FeatureId) -> String {
    inputs
        .document
        .feature(feature)
        .map(|feature| feature.name.clone())
        .unwrap_or_default()
}

struct Ends<'a> {
    context: &'a Context<'a>,
    inputs: &'a Inputs<'a>,
    operation: BodyOperation,
    plane: Plane,
    regions: &'a [Region],
}

impl Ends<'_> {
    fn error(&self, reason: String, remedy: String) -> Failure {
        self.context.error(reason, remedy, self.context.own())
    }

    fn extent(&self, extent: &ExtrudeExtent) -> Result<LinearExtent, Failure> {
        let built = match extent {
            ExtrudeExtent::OneSide { end, reversed } => LinearExtent::between(
                LinearBound::Offset(0.0),
                self.bound(
                    end,
                    Side::Only {
                        reversed: *reversed,
                    },
                )?,
            ),
            ExtrudeExtent::Symmetric { distance } => {
                LinearExtent::symmetric(self.length(distance, "distance")?)
            }
            ExtrudeExtent::TwoSides { forward, backward } => LinearExtent::between(
                self.bound(backward, Side::Backward)?,
                self.bound(forward, Side::Forward)?,
            ),
        };
        built.map_err(|error| match error {
            SweepError::ZeroLength => self.error(
                "The extrusion has no length.".to_owned(),
                "Enter distances that do not cancel each other out.".to_owned(),
            ),
            other => sweep_failure(self.context, "extrusion", &other),
        })
    }

    fn length(&self, expression: &Expression, what: &str) -> Result<f64, Failure> {
        evaluate_value(
            self.context,
            expression,
            Dimension::LENGTH,
            what,
            self.inputs.parameters,
        )
        .and_then(|value| positive(self.context, value, what))
        .and_then(|value| within_reach(self.context, value, what))
    }

    fn bound(&self, end: &ExtrudeEnd, side: Side) -> Result<LinearBound, Failure> {
        match end {
            ExtrudeEnd::Distance(distance) => Ok(LinearBound::Offset(
                side.sign() * self.length(distance, side.distance())?,
            )),
            ExtrudeEnd::ThroughAll => self.through_all(side),
            ExtrudeEnd::UpToNext => self.up_to_next(side),
            ExtrudeEnd::UpToFace(target) => self.up_to_face(target, side),
        }
    }

    fn body(&self, body: FeatureId) -> Result<&Solid, Failure> {
        self.inputs
            .body(body)
            .ok_or_else(|| missing_body(self.inputs, body))
    }

    fn through_all(&self, side: Side) -> Result<LinearBound, Failure> {
        let body = match self.operation {
            BodyOperation::Remove(body) | BodyOperation::Intersect(body) => body,
            BodyOperation::NewBody | BodyOperation::Add(_) => {
                return Err(self.error(
                    "Through all only cuts into a body or intersects with it, so it cannot add \
                     material."
                        .to_owned(),
                    "Choose Remove from body or Intersect with body, or use Up to next or a \
                     distance."
                        .to_owned(),
                ));
            }
        };
        let bounds = self.body(body)?.bounding_box();
        let direction = self.plane.normal() * side.sign();
        let farthest = bounds.map_or(f64::NEG_INFINITY, |bounds| {
            bounds
                .corners()
                .iter()
                .map(|corner| (*corner - self.plane.origin()).dot(direction))
                .fold(f64::NEG_INFINITY, f64::max)
        });
        if farthest <= LINEAR_RESOLUTION {
            return Err(self.error(
                format!(
                    "No part of the body of {} lies {}, so there is nothing to go through.",
                    feature_name(self.inputs, body),
                    side.ahead()
                ),
                format!("{}, or enter a distance.", side.other_way()),
            ));
        }
        let margin =
            bounds.map_or(0.0, |bounds| bounds.diagonal()) * THROUGH_ALL_REACH + THROUGH_ALL_MARGIN;
        Ok(LinearBound::Offset(
            side.sign() * (farthest + margin).min(MAX_SIZE),
        ))
    }

    fn up_to_next(&self, side: Side) -> Result<LinearBound, Failure> {
        let Some(body) = self.operation.target() else {
            return Err(self.error(
                "Up to next stops at the body this extrusion changes, and it makes a new body \
                 instead."
                    .to_owned(),
                "Choose Add, Remove or Intersect with a body, or enter a distance.".to_owned(),
            ));
        };
        let solid = self.body(body)?;
        let found = next_face(solid, &self.plane, self.regions, side.reversed())
            .map_err(|error| self.next_failure(solid, body, side, &error))?;
        let body_name = feature_name(self.inputs, body);
        let sketch = &self.context.sketch_name;
        match (self.operation, found.entering) {
            (BodyOperation::Add(_), false) => Err(self.error(
                format!(
                    "The profile of {sketch} starts inside the body of {body_name}, so extruding \
                     it up to the next face adds nothing."
                ),
                "Choose Remove from body, or place the sketch outside the body.".to_owned(),
            )),
            (BodyOperation::Remove(_), true) => Err(self.error(
                format!(
                    "The profile of {sketch} first meets the body of {body_name} where it enters \
                     it, so extruding up to that face removes nothing."
                ),
                "Place the sketch on or inside the body, or use Through all or Up to face."
                    .to_owned(),
            )),
            _ => {
                let face = describe_origin(
                    self.inputs.document,
                    solid.face(found.face).and_then(|face| face.origin()),
                );
                self.check_ahead(found.plane, side, &face)?;
                Ok(LinearBound::Plane(found.plane))
            }
        }
    }

    fn next_failure(
        &self,
        solid: &Solid,
        body: FeatureId,
        side: Side,
        error: &ReachError,
    ) -> Failure {
        let body = feature_name(self.inputs, body);
        let sketch = &self.context.sketch_name;
        let describe = |face: FaceId| {
            describe_origin(
                self.inputs.document,
                solid.face(face).and_then(|face| face.origin()),
            )
        };
        match error {
            ReachError::Cancelled(_) => Failure::Cancelled,
            ReachError::NoRegions => self.error(
                "No region of the sketch is chosen.".to_owned(),
                "Choose at least one region.".to_owned(),
            ),
            ReachError::Nothing => self.error(
                format!(
                    "The profile of {sketch} meets nothing of the body of {body} {}.",
                    side.ahead()
                ),
                format!("{}, or enter a distance.", side.other_way()),
            ),
            ReachError::Partly => self.error(
                format!(
                    "Part of the profile of {sketch} passes beside the body of {body}, so there is \
                     no one next face to stop at."
                ),
                "Use Up to face to choose where it stops, or keep the profile within the \
                 outline of the body."
                    .to_owned(),
            ),
            ReachError::SeveralFaces(faces) => {
                let names: Vec<String> = faces.iter().map(|face| describe(*face)).collect();
                self.error(
                    format!(
                        "The profile of {sketch} meets several faces of {body} first: {}.",
                        list_names(&names)
                    ),
                    "Use Up to face to choose which one it stops at.".to_owned(),
                )
            }
            ReachError::Curved(face) => self.error(
                format!(
                    "The next face, {}, is curved, and an extrusion can only end on a flat face \
                     or plane.",
                    describe(*face)
                ),
                "Use Up to face with a flat face or plane, or enter a distance.".to_owned(),
            ),
            ReachError::Undecided => self.error(
                format!(
                    "The next face of the body of {body} {} could not be found.",
                    side.ahead()
                ),
                "Use Up to face to choose where it stops, or enter a distance.".to_owned(),
            ),
        }
    }

    fn up_to_face(&self, target: &PlaneReference, side: Side) -> Result<LinearBound, Failure> {
        let plane = self.resolve(target)?;
        self.check_ahead(plane, side, &describe_plane(self.inputs.document, target))?;
        Ok(LinearBound::Plane(plane))
    }

    fn resolve(&self, target: &PlaneReference) -> Result<Plane, Failure> {
        resolve_target(
            self.context,
            self.inputs,
            target,
            "this extrusion runs up to",
            "Select a flat face or plane and use it for this end, or choose another end.",
        )
    }

    fn check_ahead(&self, target: Plane, side: Side, name: &str) -> Result<(), Failure> {
        let found = heights(&self.plane, self.regions, &target).map_err(|error| match error {
            SweepError::EndAlongDirection => self.error(
                format!(
                    "{} runs along the direction of the extrusion, so the extrusion never \
                     reaches it.",
                    capitalized(name)
                ),
                "Choose a face or plane that faces the sketch, or enter a distance.".to_owned(),
            ),
            other => sweep_failure(self.context, "extrusion", &other),
        })?;
        let (least, most) = if side.reversed() {
            (-found.most, -found.least)
        } else {
            (found.least, found.most)
        };
        if most <= LINEAR_RESOLUTION {
            return Err(self.error(
                format!("{} does not lie {}.", capitalized(name), side.ahead()),
                format!(
                    "{}, or choose a face or plane beyond the sketch.",
                    side.other_way()
                ),
            ));
        }
        if least <= LINEAR_RESOLUTION {
            return Err(self.error(
                format!(
                    "{} cuts across the profile of {}, so the extrusion would end before it \
                     starts in places.",
                    capitalized(name),
                    self.context.sketch_name
                ),
                "Choose a face or plane that lies wholly beyond the profile.".to_owned(),
            ));
        }
        Ok(())
    }
}

fn angular_extent(
    context: &Context<'_>,
    extent: &RevolveExtent,
    parameters: &ParameterValues,
) -> Result<AngularExtent, Failure> {
    let angle = |expression: &Expression, what: &str| {
        evaluate_value(context, expression, Dimension::ANGLE, what, parameters)
            .and_then(|degrees| positive(context, degrees, what))
            .map(f64::to_radians)
    };
    let built = match extent {
        RevolveExtent::Full => Ok(AngularExtent::full()),
        RevolveExtent::OneSide {
            angle: value,
            reversed,
        } => {
            let radians = angle(value, "angle")?;
            AngularExtent::one_side(if *reversed { -radians } else { radians })
        }
        RevolveExtent::Symmetric { angle: value } => {
            AngularExtent::symmetric(angle(value, "angle")?)
        }
        RevolveExtent::TwoSides { forward, backward } => {
            let forward = angle(forward, "forward angle")?;
            let backward = angle(backward, "backward angle")?;
            return AngularExtent::new(-backward, forward).map_err(|error| match error {
                SweepError::BeyondFullTurn => context.error(
                    "The two angles add up to more than 360°.".to_owned(),
                    "Enter angles that add up to at most 360°.".to_owned(),
                    context.own(),
                ),
                other => sweep_failure(context, "revolution", &other),
            });
        }
    };
    built.map_err(|error| sweep_failure(context, "revolution", &error))
}

fn model_axis(
    context: &Context<'_>,
    feature: &Feature,
    inputs: &Inputs<'_>,
    plane: &Plane,
    reference: &AxisReference,
) -> Result<Axis2, Failure> {
    let axis = Resolver { feature, inputs }.axis(reference)?;
    let in_plane = tolerance::along_plane(axis, plane);
    let origin = plane.to_local(axis.origin());
    let direction = plane.to_local(axis.origin() + axis.direction()) - origin;
    match Axis2::new(origin, direction) {
        Ok(found) if in_plane => Ok(found),
        _ => Err(context.error(
            format!(
                "{} does not lie in the plane of {}, so the sketch cannot turn about it.",
                capitalized(&describe_axis(inputs.document, reference)),
                context.sketch_name
            ),
            "Choose an axis that lies in the sketch plane, or a line of the sketch.".to_owned(),
            context.own(),
        )),
    }
}

fn revolution_axis(context: &Context<'_>, axis: EntityId) -> Result<Axis2, Failure> {
    let sketch = context.sketch;
    let found = match axis.reference() {
        Some(Reference::HorizontalAxis) => Axis2::new(Point2::ZERO, Vector2::X).ok(),
        Some(Reference::VerticalAxis) => Axis2::new(Point2::ZERO, Vector2::Y).ok(),
        Some(Reference::Origin) => None,
        None => sketch
            .line_endpoints(axis)
            .and_then(|(start, end)| Axis2::through(start, end).ok()),
    };
    found.ok_or_else(|| {
        context.error(
            format!(
                "The revolution axis, {}, is not a line of {}.",
                sketch.entity_label(axis),
                context.sketch_name
            ),
            "Choose a line or one of the sketch axes as the revolution axis.".to_owned(),
            context.own(),
        )
    })
}
