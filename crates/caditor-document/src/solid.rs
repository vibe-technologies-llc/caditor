use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet},
    f64::consts::PI,
    panic::{self, AssertUnwindSafe},
    sync::{Arc, OnceLock},
};

use caditor_expression::{Dimension, Expression, ParameterId};
use caditor_geometry::{Aabb, Aabb2, Plane, Point2, Ray, Vector2, Vector3};
use caditor_kernel::{
    Accuracy, AngularExtent, Axis2, BooleanError, BooleanOperation, DisplayMesh, EdgeId, EdgeName,
    FaceId, FaceName, FaceOrigin, GeometryError, Heights, LINEAR_RESOLUTION, LinearBound,
    LinearExtent, MAX_SIZE, MAX_TAPER_DEGREES, MassProperties, MeasureError, Mesh, MeshQuality,
    MeshedSolid, OpenEnd, Profile, ProfileCurve, ProfileError, ReachError, ReferenceError, Region,
    RegionMesh, RegionReference, SamplingTolerance, Selection, Solid, StopError, SweepError,
    TessellationError, VertexId, VertexName, WallError, WallSide, boolean, extrude_along,
    extrude_tapered, heights, heights_along, mass_properties_tessellating, next_face,
    resolve_regions, revolve, stop_at_body, vertex_names, wall_regions,
};
use caditor_sketch::{Entity, EntityId, Reference, Sketch};

use crate::{
    attachment::{FaceAttachment, face_plane},
    blend::BlendRecipe,
    body_pieces,
    datum::{
        AxisReference, PlaneReference, Resolver, capitalized, describe_axis, describe_plane,
        displayed_plane,
    },
    describe::describe_origin,
    document::{Feature, FeatureId},
    failure::{Cause, Expected, Upstream},
    recompute::{
        CancelToken, Evaluation, Failure, FeatureError, FeatureResult, FixTarget, Inputs,
        MissingInput, SketchResult,
    },
    shell::ShellRecipe,
    solid_cause::{CurveProblem, Gap, SolidCause, SquareEnd, TargetFor},
    sweep::{SWEEP_SHAPE, Sweep, SweepPath, swept},
    tolerance,
    trouble::{self, Adjust, boolean_trouble},
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
    UpToNext {
        offset: Option<Box<Expression>>,
    },
    UpToFace {
        target: Box<PlaneReference>,
        offset: Option<Box<Expression>>,
    },
    UpToSurface {
        face: Box<FaceAttachment>,
    },
}

impl ExtrudeEnd {
    pub fn up_to_next() -> Self {
        Self::UpToNext { offset: None }
    }

    pub fn up_to_face(target: PlaneReference) -> Self {
        Self::UpToFace {
            target: Box::new(target),
            offset: None,
        }
    }

    pub fn up_to_surface(face: FaceAttachment) -> Self {
        Self::UpToSurface {
            face: Box::new(face),
        }
    }

    pub fn distance(&self) -> Option<&Expression> {
        match self {
            Self::Distance(distance) => Some(distance),
            Self::ThroughAll
            | Self::UpToNext { .. }
            | Self::UpToFace { .. }
            | Self::UpToSurface { .. } => None,
        }
    }

    pub fn target(&self) -> Option<&PlaneReference> {
        match self {
            Self::UpToFace { target, .. } => Some(target),
            Self::Distance(_)
            | Self::ThroughAll
            | Self::UpToNext { .. }
            | Self::UpToSurface { .. } => None,
        }
    }

    pub fn surface(&self) -> Option<&FaceAttachment> {
        match self {
            Self::UpToSurface { face } => Some(face),
            Self::Distance(_)
            | Self::ThroughAll
            | Self::UpToNext { .. }
            | Self::UpToFace { .. } => None,
        }
    }

    pub fn offset(&self) -> Option<&Expression> {
        match self {
            Self::UpToNext { offset } | Self::UpToFace { offset, .. } => offset.as_deref(),
            Self::Distance(_) | Self::ThroughAll | Self::UpToSurface { .. } => None,
        }
    }

    pub fn expressions_mut(&mut self) -> Vec<&mut Expression> {
        match self {
            Self::Distance(distance) => vec![distance],
            Self::UpToNext { offset } | Self::UpToFace { offset, .. } => {
                offset.as_deref_mut().into_iter().collect()
            }
            Self::ThroughAll | Self::UpToSurface { .. } => Vec::new(),
        }
    }

    pub fn takes_offset(&self) -> bool {
        matches!(self, Self::UpToNext { .. } | Self::UpToFace { .. })
    }

    #[must_use]
    pub fn with_offset(self, offset: Option<Expression>) -> Self {
        let offset = offset.map(Box::new);
        match self {
            Self::UpToNext { .. } => Self::UpToNext { offset },
            Self::UpToFace { target, .. } => Self::UpToFace { target, offset },
            other @ (Self::Distance(_) | Self::ThroughAll | Self::UpToSurface { .. }) => other,
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

    pub fn ends_mut(&mut self) -> Vec<&mut ExtrudeEnd> {
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
                .flat_map(|end| end.distance().into_iter().chain(end.offset()))
                .collect(),
        }
    }

    fn targets(&self) -> Vec<&PlaneReference> {
        self.ends()
            .into_iter()
            .filter_map(ExtrudeEnd::target)
            .collect()
    }

    pub fn surfaces(&self) -> Vec<&FaceAttachment> {
        self.ends()
            .into_iter()
            .filter_map(ExtrudeEnd::surface)
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
    UpTo {
        target: Box<PlaneReference>,
        reversed: bool,
    },
}

impl RevolveExtent {
    pub fn up_to(target: PlaneReference, reversed: bool) -> Self {
        Self::UpTo {
            target: Box::new(target),
            reversed,
        }
    }

    pub fn target(&self) -> Option<&PlaneReference> {
        match self {
            Self::UpTo { target, .. } => Some(target),
            Self::Full | Self::OneSide { .. } | Self::Symmetric { .. } | Self::TwoSides { .. } => {
                None
            }
        }
    }

    fn expressions(&self) -> Vec<&Expression> {
        match self {
            Self::Full | Self::UpTo { .. } => Vec::new(),
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
pub struct Wall {
    pub thickness: Expression,
    pub side: WallSide,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Extrude {
    pub sketch: FeatureId,
    pub regions: RegionChoice,
    pub extent: ExtrudeExtent,
    pub operation: BodyOperation,
    pub start: Option<SolidStart>,
    pub other_bodies: Vec<FeatureId>,
    pub taper: Option<Box<Expression>>,
    pub wall: Option<Box<Wall>>,
    pub direction: Option<Box<AxisReference>>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AxisSide {
    Left,
    Right,
}

impl AxisSide {
    pub fn other(self) -> Self {
        match self {
            Self::Left => Self::Right,
            Self::Right => Self::Left,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
        }
    }

    fn holds(self, signed_distance: f64) -> bool {
        match self {
            Self::Left => signed_distance > 0.0,
            Self::Right => signed_distance < 0.0,
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
    pub other_bodies: Vec<FeatureId>,
    pub side: Option<AxisSide>,
    pub wall: Option<Box<Wall>>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SolidFeature {
    Extrude(Extrude),
    Revolve(Revolve),
    Sweep(Sweep),
}

impl SolidFeature {
    pub fn sketch(&self) -> FeatureId {
        match self {
            Self::Extrude(extrude) => extrude.sketch,
            Self::Revolve(revolve) => revolve.sketch,
            Self::Sweep(sweep) => sweep.sketch,
        }
    }

    pub fn regions(&self) -> &RegionChoice {
        match self {
            Self::Extrude(extrude) => &extrude.regions,
            Self::Revolve(revolve) => &revolve.regions,
            Self::Sweep(sweep) => &sweep.regions,
        }
    }

    pub fn operation(&self) -> BodyOperation {
        match self {
            Self::Extrude(extrude) => extrude.operation,
            Self::Revolve(revolve) => revolve.operation,
            Self::Sweep(sweep) => sweep.operation,
        }
    }

    pub fn other_bodies(&self) -> &[FeatureId] {
        match self {
            Self::Extrude(extrude) => &extrude.other_bodies,
            Self::Revolve(revolve) => &revolve.other_bodies,
            Self::Sweep(sweep) => &sweep.other_bodies,
        }
    }

    pub fn other_bodies_mut(&mut self) -> &mut Vec<FeatureId> {
        match self {
            Self::Extrude(extrude) => &mut extrude.other_bodies,
            Self::Revolve(revolve) => &mut revolve.other_bodies,
            Self::Sweep(sweep) => &mut sweep.other_bodies,
        }
    }

    pub fn target_bodies(&self) -> Vec<FeatureId> {
        self.operation()
            .target()
            .into_iter()
            .chain(self.other_bodies().iter().copied())
            .collect()
    }

    pub fn axis(&self) -> Option<&RevolveAxis> {
        match self {
            Self::Extrude(_) | Self::Sweep(_) => None,
            Self::Revolve(revolve) => Some(&revolve.axis),
        }
    }

    pub fn direction(&self) -> Option<&AxisReference> {
        match self {
            Self::Extrude(extrude) => extrude.direction.as_deref(),
            Self::Revolve(_) | Self::Sweep(_) => None,
        }
    }

    pub fn model_axis(&self) -> Option<&AxisReference> {
        self.axis()
            .and_then(RevolveAxis::model)
            .or_else(|| self.direction())
    }

    pub fn axis_line(&self) -> Option<EntityId> {
        self.axis().and_then(RevolveAxis::line)
    }

    pub fn axis_body(&self) -> Option<FeatureId> {
        self.model_axis().and_then(AxisReference::body)
    }

    pub fn axis_datum(&self) -> Option<FeatureId> {
        self.model_axis().and_then(AxisReference::datum)
    }

    pub fn start(&self) -> Option<&SolidStart> {
        match self {
            Self::Extrude(extrude) => extrude.start.as_ref(),
            Self::Revolve(revolve) => revolve.start.as_ref(),
            Self::Sweep(_) => None,
        }
    }

    pub fn wall(&self) -> Option<&Wall> {
        match self {
            Self::Extrude(extrude) => extrude.wall.as_deref(),
            Self::Revolve(revolve) => revolve.wall.as_deref(),
            Self::Sweep(_) => None,
        }
    }

    pub fn taper(&self) -> Option<&Expression> {
        match self {
            Self::Extrude(extrude) => extrude.taper.as_deref(),
            Self::Revolve(_) | Self::Sweep(_) => None,
        }
    }

    fn targets(&self) -> Vec<&PlaneReference> {
        let ends = match self {
            Self::Extrude(extrude) => extrude.extent.targets(),
            Self::Revolve(revolve) => revolve.extent.target().into_iter().collect(),
            Self::Sweep(_) => Vec::new(),
        };
        ends.into_iter()
            .chain(self.start().and_then(SolidStart::target))
            .collect()
    }

    fn surfaces(&self) -> Vec<&FaceAttachment> {
        match self {
            Self::Extrude(extrude) => extrude.extent.surfaces(),
            Self::Revolve(_) | Self::Sweep(_) => Vec::new(),
        }
    }

    pub fn end_bodies(&self) -> BTreeSet<FeatureId> {
        self.targets()
            .into_iter()
            .filter_map(PlaneReference::body)
            .chain(self.surfaces().into_iter().map(|face| face.body))
            .collect()
    }

    pub fn origin_features(&self) -> BTreeSet<FeatureId> {
        self.targets()
            .into_iter()
            .flat_map(PlaneReference::origin_features)
            .chain(
                self.surfaces()
                    .into_iter()
                    .flat_map(FaceAttachment::origin_features),
            )
            .chain(
                self.model_axis()
                    .into_iter()
                    .flat_map(AxisReference::origin_features),
            )
            .chain(
                self.path()
                    .map(SweepPath::origin_features)
                    .unwrap_or_default(),
            )
            .collect()
    }

    pub fn frames(&self) -> BTreeSet<FeatureId> {
        self.targets()
            .into_iter()
            .filter_map(PlaneReference::frame)
            .chain(self.model_axis().and_then(AxisReference::frame))
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
            Self::Sweep(_) => SWEEP_SHAPE,
        }
    }

    pub fn same_kind(&self, other: &Self) -> bool {
        matches!(
            (self, other),
            (Self::Extrude(_), Self::Extrude(_))
                | (Self::Revolve(_), Self::Revolve(_))
                | (Self::Sweep(_), Self::Sweep(_))
        )
    }

    fn expressions(&self) -> Vec<&Expression> {
        match self {
            Self::Extrude(extrude) => {
                let mut expressions = extrude.extent.expressions();
                expressions.extend(self.start().and_then(SolidStart::distance));
                expressions.extend(self.taper());
                expressions.extend(self.wall().map(|wall| &wall.thickness));
                expressions
            }
            Self::Revolve(revolve) => {
                let mut expressions = revolve.extent.expressions();
                expressions.extend(self.start().and_then(SolidStart::distance));
                expressions.extend(self.wall().map(|wall| &wall.thickness));
                expressions
            }
            Self::Sweep(sweep) => sweep.expressions(),
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
            .sum::<usize>()
            + self
                .surfaces()
                .into_iter()
                .map(|face| size_of::<FaceAttachment>() + face.face.heap_size())
                .sum::<usize>();
        let axis = match self {
            Self::Extrude(extrude) => extrude
                .direction
                .as_deref()
                .map_or(0, |axis| size_of::<AxisReference>() + axis.heap_size()),
            Self::Revolve(revolve) => match &revolve.axis {
                RevolveAxis::Model(axis) => axis.heap_size(),
                RevolveAxis::Sketch(_) => 0,
            },
            Self::Sweep(sweep) => sweep.path.heap_size(),
        };
        let references = planes + axis;
        chosen + expressions + references + size_of_val(self.other_bodies())
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

    pub fn axis_sketch(&self) -> Option<FeatureId> {
        self.model_axis().and_then(AxisReference::sketch)
    }

    pub fn path(&self) -> Option<&SweepPath> {
        match self {
            Self::Sweep(sweep) => Some(&sweep.path),
            Self::Extrude(_) | Self::Revolve(_) => None,
        }
    }

    pub fn path_body(&self) -> Option<FeatureId> {
        self.path().and_then(SweepPath::body)
    }

    pub fn reference_sketches(&self) -> BTreeSet<FeatureId> {
        self.axis_sketch()
            .into_iter()
            .chain(self.path().and_then(SweepPath::sketch))
            .collect()
    }

    pub fn features(&self) -> BTreeSet<FeatureId> {
        std::iter::once(self.sketch())
            .chain(self.target_bodies())
            .chain(self.reference_sketches())
            .chain(self.path_body())
            .chain(self.path().and_then(SweepPath::helix))
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReachedSide {
    Only,
    Forward,
    Backward,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReachedEnd {
    pub side: ReachedSide,
    pub plane: Plane,
}

impl ReachedEnd {
    pub fn level_over(&self, sketch_plane: &Plane) -> Option<f64> {
        tolerance::parallel(self.plane.normal(), sketch_plane.normal())
            .then(|| sketch_plane.signed_distance(self.plane.origin()))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SolidResult {
    pub body: FeatureId,
    pub solid: Solid,
    reached: Vec<ReachedEnd>,
    others: Vec<Arc<FeatureResult>>,
    cuts: Vec<Arc<FeatureResult>>,
    joins: Vec<Arc<FeatureResult>>,
    blended: Option<Arc<BlendRecipe>>,
    shelled: Option<Arc<ShellRecipe>>,
    mesh: OnceLock<Option<DisplayMesh>>,
    bounds: OnceLock<Option<Aabb>>,
    names: OnceLock<NameIndex>,
    mass: OnceLock<Box<Result<ExactMass, MeasureError>>>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExactMass {
    pub properties: MassProperties,
    pub accuracy: Accuracy,
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
            reached: Vec::new(),
            others: Vec::new(),
            cuts: Vec::new(),
            joins: Vec::new(),
            blended: None,
            shelled: None,
            mesh: OnceLock::new(),
            bounds: OnceLock::new(),
            names: OnceLock::new(),
            mass: OnceLock::new(),
        }
    }

    #[must_use]
    pub fn reaching(mut self, reached: Vec<ReachedEnd>) -> Self {
        self.reached = reached;
        self
    }

    pub fn reached(&self) -> &[ReachedEnd] {
        &self.reached
    }

    pub fn cutting(mut self, tools: impl IntoIterator<Item = Solid>) -> Self {
        let body = self.body;
        self.cuts = tools
            .into_iter()
            .map(|tool| Arc::new(FeatureResult::Solid(SolidResult::new(body, tool))))
            .collect();
        self
    }

    pub fn cuts(&self) -> &[Arc<FeatureResult>] {
        &self.cuts
    }

    pub fn joining(mut self, tools: impl IntoIterator<Item = Solid>) -> Self {
        let body = self.body;
        self.joins = tools
            .into_iter()
            .map(|tool| Arc::new(FeatureResult::Solid(SolidResult::new(body, tool))))
            .collect();
        self
    }

    pub fn joins(&self) -> &[Arc<FeatureResult>] {
        &self.joins
    }

    #[must_use]
    pub fn blending(mut self, recipe: BlendRecipe) -> Self {
        self.blended = Some(Arc::new(recipe));
        self
    }

    pub fn blended(&self) -> Option<&BlendRecipe> {
        self.blended.as_deref()
    }

    #[must_use]
    pub fn shelling(mut self, recipe: ShellRecipe) -> Self {
        self.shelled = Some(Arc::new(recipe));
        self
    }

    pub fn shelled(&self) -> Option<&ShellRecipe> {
        self.shelled.as_deref()
    }

    pub fn with_others(mut self, others: impl IntoIterator<Item = SolidResult>) -> Self {
        self.others = others
            .into_iter()
            .map(|other| Arc::new(FeatureResult::Solid(other)))
            .collect();
        self
    }

    pub fn others(&self) -> &[Arc<FeatureResult>] {
        &self.others
    }

    pub(crate) fn same_shapes(&self, other: &Self) -> bool {
        self.body == other.body
            && self.solid == other.solid
            && same_parts(&self.others, &other.others)
            && same_parts(&self.cuts, &other.cuts)
            && same_parts(&self.joins, &other.joins)
            && self.blended == other.blended
            && self.shelled == other.shelled
    }

    pub fn names(&self) -> &NameIndex {
        self.names.get_or_init(|| NameIndex::of(&self.solid))
    }

    pub fn bounding_box(&self) -> Option<Aabb> {
        *self.bounds.get_or_init(|| self.solid.bounding_box())
    }

    pub fn mesh(&self) -> Option<&Mesh> {
        self.display_mesh().map(DisplayMesh::mesh)
    }

    pub fn exact_mass(&self) -> Result<ExactMass, MeasureError> {
        if let Some(found) = self.mass.get() {
            return **found;
        }
        let tolerance = self.solid.tolerance_for(&MeshQuality::SMOOTH);
        let found = mass_properties_tessellating(&self.solid, &tolerance).map(|mass| ExactMass {
            properties: mass.properties,
            accuracy: if mass.meshed_faces.is_empty() {
                Accuracy::Exact
            } else {
                Accuracy::Approximate
            },
        });
        if !matches!(found, Err(MeasureError::Cancelled(_))) {
            let _ = self.mass.set(Box::new(found));
        }
        found
    }

    pub fn is_mass_known(&self) -> bool {
        self.mass.get().is_some()
    }

    pub(crate) fn display_mesh(&self) -> Option<&DisplayMesh> {
        self.mesh.get().and_then(Option::as_ref)
    }

    pub fn is_meshed(&self) -> bool {
        self.mesh.get().is_some()
    }

    pub fn mesh_failed(&self) -> bool {
        matches!(self.mesh.get(), Some(None))
    }

    pub(crate) fn tessellate(&self, name: &str, quality: &MeshQuality, earlier: Option<&Self>) {
        if self.is_meshed() {
            return;
        }
        let earlier = earlier.and_then(|earlier| {
            Some(MeshedSolid {
                solid: &earlier.solid,
                mesh: earlier.display_mesh()?,
            })
        });
        let tessellated = panic::catch_unwind(AssertUnwindSafe(|| {
            self.bounding_box();
            self.solid.display_mesh_reusing(quality, earlier)
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

pub fn body_parts(result: &Arc<FeatureResult>) -> impl Iterator<Item = &Arc<FeatureResult>> {
    std::iter::once(result).chain(result.solid().map_or(&[][..], SolidResult::others))
}

pub fn body_part(result: &Arc<FeatureResult>, body: FeatureId) -> Option<&Arc<FeatureResult>> {
    body_parts(result).find(|part| part.solid().is_some_and(|solid| solid.body == body))
}

pub(crate) fn profile_curves(sketch: &Sketch) -> Vec<ProfileCurve> {
    sketch
        .entities()
        .filter(|(id, _)| !sketch.is_construction(*id))
        .filter_map(|(id, _)| profile_curve(sketch, id))
        .collect()
}

pub fn profile_curve(sketch: &Sketch, id: EntityId) -> Option<ProfileCurve> {
    let raw = id.raw();
    match sketch.entity(id)? {
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
        Entity::Spline { .. } => sketch.spline(id).map(|spline| match spline.weights() {
            Some(weights) => ProfileCurve::rational_spline(
                raw,
                spline.degree(),
                spline.knots().to_vec(),
                spline.control_points().to_vec(),
                weights.to_vec(),
            ),
            None => ProfileCurve::spline(
                raw,
                spline.degree(),
                spline.knots().to_vec(),
                spline.control_points().to_vec(),
            ),
        }),
        Entity::Ellipse { .. } => sketch.ellipse(id).map(|ellipse| {
            ProfileCurve::ellipse(raw, ellipse.center, ellipse.major, ellipse.minor_radius)
        }),
        Entity::EllipticalArc { start, end, .. } => {
            let ellipse = sketch.ellipse(id)?;
            Some(ProfileCurve::elliptical_arc(
                raw,
                ellipse.center,
                ellipse.major,
                ellipse.minor_radius,
                (sketch.point(*start)?, sketch.point(*end)?),
            ))
        }
    }
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

pub(crate) struct Context<'a> {
    pub(crate) feature: &'a Feature,
    pub(crate) sketch_name: String,
    sketch_id: FeatureId,
    sketch: &'a Sketch,
}

impl<'a> Context<'a> {
    pub(crate) fn of_sketch(
        feature: &'a Feature,
        sketch_name: String,
        sketch_id: FeatureId,
        sketch: &'a Sketch,
    ) -> Self {
        Self {
            feature,
            sketch_name,
            sketch_id,
            sketch,
        }
    }

    pub(crate) fn error(&self, cause: impl Into<Cause>) -> Failure {
        Failure::from(FeatureError::new(cause).fixing(self.own()))
    }

    pub(crate) fn error_in_sketch(&self, cause: impl Into<Cause>) -> Failure {
        Failure::from(FeatureError::new(cause).fixing(self.in_sketch()))
    }

    pub(crate) fn own(&self) -> FixTarget {
        FixTarget::Feature(self.feature.id())
    }

    pub(crate) fn value(
        &self,
        expression: &Expression,
        dimension: Dimension,
        what: &str,
        parameters: &ParameterValues,
    ) -> Result<f64, Failure> {
        evaluate_value(self, expression, dimension, what, parameters)
    }

    pub(crate) fn in_sketch(&self) -> FixTarget {
        FixTarget::Feature(self.sketch_id)
    }

    pub(crate) fn no_regions(&self) -> Failure {
        Failure::Waiting(MissingInput::Regions {
            sketch: self.sketch_id,
        })
    }

    fn curves(&self, entities: &[u64]) -> String {
        let labels: Vec<String> = entities
            .iter()
            .map(|raw| match *raw {
                REVOLUTION_AXIS_ENTITY => "the revolution axis".to_owned(),
                raw => self.sketch.entity_label(EntityId::from_raw(raw)),
            })
            .collect();
        crate::document::list_names(&labels)
    }
}

fn same_parts(own: &[Arc<FeatureResult>], theirs: &[Arc<FeatureResult>]) -> bool {
    own.len() == theirs.len()
        && own
            .iter()
            .zip(theirs)
            .all(|(own, theirs)| match (own.solid(), theirs.solid()) {
                (Some(own), Some(theirs)) => own.body == theirs.body && own.solid == theirs.solid,
                _ => false,
            })
}

pub(crate) fn evaluate(
    feature: &Feature,
    solid: &SolidFeature,
    inputs: &Inputs<'_>,
    cancel: &CancelToken,
) -> Result<FeatureResult, Failure> {
    let sketch_id = solid.sketch();
    let sketch_name = inputs.document.body_label(sketch_id).unwrap_or_default();
    let Some(FeatureResult::Sketch(sketch)) = inputs.features.get(&sketch_id).map(AsRef::as_ref)
    else {
        return Err(Failure::from(
            FeatureError::new(Upstream::NoShape { name: sketch_name })
                .fixing(FixTarget::Feature(sketch_id)),
        ));
    };
    let context = Context {
        feature,
        sketch_name,
        sketch_id,
        sketch: &sketch.geometry,
    };
    let regions = match solid.wall() {
        Some(wall) => walled_regions(&context, &sketch.geometry, wall, inputs.parameters)?,
        None => chosen_regions(&context, sketch, solid.regions())?,
    };
    if cancel.is_cancelled() {
        return Err(Failure::Cancelled);
    }
    let plane = sketch.geometry.plane();
    let raw = feature.id().raw();
    let started = match solid.start() {
        Some(start) => start_plane(&context, inputs, solid.shape(), plane, start)?,
        None => plane,
    };
    let mut stopped = None;
    let mut reached = Vec::new();
    let tool = match solid {
        SolidFeature::Extrude(definition) => {
            let plane = started;
            let along = match definition.direction.as_deref() {
                Some(reference) => Some(extrusion_direction(
                    &context, feature, inputs, &plane, reference,
                )?),
                None => None,
            };
            let ends = Ends {
                context: &context,
                inputs,
                operation: definition.operation,
                other_bodies: &definition.other_bodies,
                plane,
                regions: &regions,
                stop: Cell::new(None),
                reached: RefCell::new(Vec::new()),
                along,
            };
            let (extent, stop) = ends.extent(&definition.extent)?;
            stopped = stop;
            reached = ends.reached.take();
            let taper = match definition.taper.as_deref() {
                Some(angle) => taper_angle(&context, angle, inputs.parameters)?,
                None => 0.0,
            };
            match along {
                Some(_) if taper != 0.0 => {
                    return Err(context.error(SolidCause::TaperWithDirection {
                        feature: feature.name.clone(),
                    }));
                }
                Some(direction) => extrude_along(&plane, &regions, extent, direction, raw),
                None => extrude_tapered(&plane, &regions, extent, taper, raw),
            }
        }
        SolidFeature::Revolve(definition) => {
            let axis = match &definition.axis {
                RevolveAxis::Sketch(line) => revolution_axis(&context, *line)?,
                RevolveAxis::Model(reference) => {
                    model_axis(&context, feature, inputs, &plane, reference)?
                }
            };
            let kept;
            let turned: &[Region] = match (definition.side, &definition.wall) {
                (Some(_), Some(_)) => {
                    return Err(context.error(SolidCause::WallWithSide));
                }
                (Some(side), None) => {
                    kept = one_side(&context, sketch, &regions, axis, side)?;
                    &kept
                }
                (None, _) => &regions,
            };
            let extent = match &definition.extent {
                RevolveExtent::UpTo { target, reversed } => {
                    let turn = Turn {
                        context: &context,
                        inputs,
                        plane: &started,
                        axis,
                        regions: turned,
                    };
                    turn.up_to(target, *reversed)?
                }
                other => angular_extent(&context, other, inputs.parameters)?,
            };
            revolve(&started, turned, axis, extent, raw)
        }
        SolidFeature::Sweep(definition) => {
            Ok(swept(&context, inputs, (&started, &regions), definition)?)
        }
    }
    .map_err(|error| sweep_failure(&context, solid.shape(), &error))?;
    let tool = match stopped {
        Some(stop) => {
            let stopping = Stopping {
                context: &context,
                inputs,
                operation: solid.operation(),
                plane: &started,
                regions: &regions,
            };
            stopping.trim(tool, stop)?
        }
        None => tool,
    };
    if cancel.is_cancelled() {
        return Err(Failure::Cancelled);
    }
    let other_bodies = solid.other_bodies();
    let operation = solid.operation();
    let (body, solid, cut) = match operation {
        BodyOperation::NewBody => (feature.id(), tool, None),
        operation @ (BodyOperation::Add(body)
        | BodyOperation::Remove(body)
        | BodyOperation::Intersect(body)) => {
            let current = inputs.body(body).ok_or_else(|| inputs.missing_body(body))?;
            let kernel_operation = match operation {
                BodyOperation::Remove(_) => BooleanOperation::Difference,
                BodyOperation::Intersect(_) => BooleanOperation::Intersection,
                BodyOperation::NewBody | BodyOperation::Add(_) => BooleanOperation::Union,
            };
            let combined = boolean(current, &tool, kernel_operation).map_err(|error| {
                boolean_failure(&context, inputs, [current, &tool], body, operation, &error)
            })?;
            (body, combined, Some(tool))
        }
    };
    let mut distinct = Vec::new();
    for other in other_bodies {
        if *other != body && !distinct.contains(other) {
            distinct.push(*other);
        }
    }
    let (cut, joined) = match operation {
        BodyOperation::Remove(_) => (cut, None),
        BodyOperation::Add(_) => (None, cut),
        BodyOperation::NewBody | BodyOperation::Intersect(_) => (None, None),
    };
    let others = match (&cut, distinct.as_slice()) {
        (Some(tool), others) => cut_others(&context, inputs, tool, others, cancel)?,
        (None, []) => Vec::new(),
        (None, [_, ..]) => {
            return Err(context.error(SolidCause::OnlyRemovalCutsSeveral));
        }
    };
    if operation == BodyOperation::NewBody && feature.separates_pieces() {
        let mut pieces = body_pieces::separated(feature, &solid)?
            .into_iter()
            .map(|(piece, lump)| SolidResult::new(piece, lump));
        if let Some(first) = pieces.next() {
            return Ok(FeatureResult::Solid(
                first.reaching(reached).with_others(pieces),
            ));
        }
    }
    Ok(FeatureResult::Solid(
        SolidResult::new(body, solid)
            .reaching(reached)
            .with_others(others)
            .cutting(cut)
            .joining(joined),
    ))
}

fn cut_others(
    context: &Context<'_>,
    inputs: &Inputs<'_>,
    tool: &Solid,
    others: &[FeatureId],
    cancel: &CancelToken,
) -> Result<Vec<SolidResult>, Failure> {
    let operation = BodyOperation::Remove(context.feature.id());
    others
        .iter()
        .map(|other| {
            if cancel.is_cancelled() {
                return Err(Failure::Cancelled);
            }
            let current = inputs
                .body(*other)
                .ok_or_else(|| inputs.missing_body(*other))?;
            let cut = boolean(current, tool, BooleanOperation::Difference).map_err(|error| {
                boolean_failure(
                    context,
                    inputs,
                    [current, tool],
                    *other,
                    operation.with_target(*other),
                    &error,
                )
            })?;
            Ok(SolidResult::new(*other, cut))
        })
        .collect()
}

fn resolve_target(
    context: &Context<'_>,
    inputs: &Inputs<'_>,
    target: &PlaneReference,
    purpose: TargetFor,
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
        .ok_or_else(|| inputs.missing_body(attachment.body))?;
    attachment.resolve(solid).map_err(|error| {
        context.error(SolidCause::Target {
            error,
            body,
            target: purpose,
        })
    })
}

fn start_plane(
    context: &Context<'_>,
    inputs: &Inputs<'_>,
    shape: &'static str,
    plane: Plane,
    start: &SolidStart,
) -> Result<Plane, Failure> {
    match start {
        SolidStart::Distance(offset) => offset_plane(context, plane, offset, inputs.parameters),
        SolidStart::Plane(target) => {
            let found = resolve_target(context, inputs, target, TargetFor::Start { shape })?;
            if !tolerance::parallel(found.normal(), plane.normal()) {
                return Err(context.error(SolidCause::StartNotParallel {
                    plane: capitalized(&describe_plane(inputs.document, target)),
                    sketch: context.sketch_name.clone(),
                    shape,
                }));
            }
            let distance = plane.signed_distance(found.origin());
            within_reach(context, distance.abs(), "start offset")?;
            Plane::from_frame(
                plane.origin() + plane.normal() * distance,
                plane.normal(),
                plane.x_axis(),
            )
            .ok_or_else(|| {
                context.error(SolidCause::StartUnplaceable {
                    shape,
                    sketch: context.sketch_name.clone(),
                })
            })
        }
    }
}

pub fn displayed_start_offset(
    evaluation: &Evaluation,
    user: FeatureId,
    sketch_plane: &Plane,
    target: &PlaneReference,
) -> Option<f64> {
    let found = displayed_plane(evaluation, user, target)?;
    tolerance::parallel(found.normal(), sketch_plane.normal())
        .then(|| sketch_plane.signed_distance(found.origin()))
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
        context.error(SolidCause::OffsetUnapplicable {
            sketch: context.sketch_name.clone(),
        })
    })
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

fn walled_regions(
    context: &Context<'_>,
    sketch: &Sketch,
    wall: &Wall,
    parameters: &ParameterValues,
) -> Result<Vec<Region>, Failure> {
    const WHAT: &str = "wall thickness";
    let thickness = evaluate_value(
        context,
        &wall.thickness,
        Dimension::LENGTH,
        WHAT,
        parameters,
    )
    .and_then(|value| positive(context, value, WHAT))
    .and_then(|value| within_reach(context, value, WHAT))?;
    wall_regions(&profile_curves(sketch), thickness, wall.side)
        .map_err(|error| wall_failure(context, &error))
}

fn taper_angle(
    context: &Context<'_>,
    angle: &Expression,
    parameters: &ParameterValues,
) -> Result<f64, Failure> {
    let degrees = evaluate_value(context, angle, Dimension::ANGLE, "taper angle", parameters)?;
    if degrees.abs() >= MAX_TAPER_DEGREES {
        return Err(steep_taper(context));
    }
    Ok(degrees.to_radians())
}

fn steep_taper(context: &Context<'_>) -> Failure {
    context.error(SolidCause::SteepTaper {
        feature: context.feature.name.clone(),
    })
}

fn wall_failure(context: &Context<'_>, error: &WallError) -> Failure {
    let sketch = context.sketch_name.clone();
    let feature = context.feature.name.clone();
    let curves = context.curves(&error.entities());
    match error {
        WallError::Cancelled(_) => Failure::Cancelled,
        WallError::Curve(error) => profile_failure(context, error),
        WallError::NoCurves => context.error_in_sketch(SolidCause::WallNoCurves { sketch }),
        WallError::NonFinite | WallError::NotPositive => context.error(SolidCause::WallNotPositive),
        WallError::BeyondMaximum => context.error(SolidCause::WallTooBig),
        WallError::Branches { .. } => {
            context.error_in_sketch(SolidCause::WallBranches { sketch, curves })
        }
        WallError::Folds { .. } => {
            context.error_in_sketch(SolidCause::WallFolds { sketch, curves })
        }
        WallError::TooThick { .. } => context.error(SolidCause::WallTooThick {
            feature,
            sketch,
            curves,
        }),
        WallError::Apart { .. } => context.error(SolidCause::WallApart {
            feature,
            sketch,
            curves,
        }),
        WallError::CrossesItself => context.error(SolidCause::WallCrosses { feature, sketch }),
        WallError::Geometry(_) => {
            log::warn!("{feature} could not be built: {error}");
            context.error(SolidCause::WallUnbuilt { sketch })
        }
    }
}

const MAX_NAMED_GAPS: usize = 2;

fn open_gaps(context: &Context<'_>, open_ends: &[OpenEnd]) -> (Vec<Gap>, usize) {
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
    let named = gaps
        .iter()
        .take(MAX_NAMED_GAPS)
        .map(|(first, second, gap)| Gap {
            first: context.curves(&[*first]),
            second: context.curves(&[*second]),
            distance: *gap,
        })
        .collect();
    (named, gaps.len().saturating_sub(MAX_NAMED_GAPS))
}

pub(crate) fn profile_failure(context: &Context<'_>, error: &ProfileError) -> Failure {
    let sketch = context.sketch_name.clone();
    match error {
        ProfileError::Cancelled(_) => Failure::Cancelled,
        ProfileError::NoClosedProfile { open_ends } => {
            let (gaps, unnamed) = open_gaps(context, open_ends);
            context.error_in_sketch(SolidCause::NoClosedShape {
                sketch,
                gaps,
                unnamed,
            })
        }
        ProfileError::EmptySelection => context.no_regions(),
        ProfileError::MissingRegion(_) => context.error(SolidCause::RegionGone { sketch }),
        ProfileError::AmbiguousRegion { candidates, .. } => {
            context.error(SolidCause::RegionAmbiguous {
                sketch,
                candidates: candidates.len(),
            })
        }
        ProfileError::Unresolved { entities } if entities.is_empty() => {
            context.error_in_sketch(SolidCause::CurvesUndivided { sketch })
        }
        ProfileError::DuplicateEntity { .. }
        | ProfileError::Degenerate { .. }
        | ProfileError::InvalidCurve { .. }
        | ProfileError::SelfOverlap { .. }
        | ProfileError::Overlap { .. }
        | ProfileError::TooIntricate { .. }
        | ProfileError::Unresolved { .. } => {
            let entities = error.entities();
            let problem = match error {
                ProfileError::Degenerate { .. } => CurveProblem::NoLength,
                ProfileError::InvalidCurve {
                    error: GeometryError::BeyondMaximum(_),
                    ..
                } => CurveProblem::BeyondModel,
                ProfileError::SelfOverlap { .. } => CurveProblem::DoublesBack,
                ProfileError::Overlap { .. } => CurveProblem::Alongside,
                ProfileError::TooIntricate { .. } => CurveProblem::TooCrossed,
                ProfileError::Unresolved { .. } => CurveProblem::Undivided,
                _ => CurveProblem::Unusable,
            };
            context.error_in_sketch(SolidCause::CurvesProblem {
                sketch,
                curves: context.curves(&entities),
                problem,
                several: entities.len() > 1,
            })
        }
    }
}

pub(crate) fn sweep_failure(context: &Context<'_>, shape: &str, error: &SweepError) -> Failure {
    let sketch = context.sketch_name.clone();
    let shape = shape.to_owned();
    match error {
        SweepError::Cancelled(_) => Failure::Cancelled,
        SweepError::NoRegions => context.no_regions(),
        SweepError::NonFinite | SweepError::ZeroLength => {
            context.error(SolidCause::NoLength { shape })
        }
        SweepError::TooLong => context.error(SolidCause::TooLong { shape }),
        SweepError::ZeroAngle => context.error(SolidCause::NoAngle),
        SweepError::BeyondFullTurn => context.error(SolidCause::TurnsTwice),
        SweepError::DirectionAlongSketch => {
            context.error(SolidCause::DirectionAlongSketch { shape, sketch })
        }
        SweepError::EndAlongDirection => context.error(SolidCause::EndAlongDirection),
        SweepError::EndsCross => context.error(SolidCause::EndsCross { sketch }),
        SweepError::DegenerateAxis => context.error(SolidCause::AxisWithoutDirection),
        SweepError::BothSidesOfAxis { left, right } => {
            let apart = if right.len() <= left.len() {
                right
            } else {
                left
            };
            context.error(SolidCause::BothSides {
                sketch,
                curves: context.curves(apart),
                several: apart.len() != 1,
            })
        }
        SweepError::CrossesAxis { .. } => context.error(SolidCause::CrossesAxis {
            sketch,
            curves: context.curves(&error.entities()),
        }),
        SweepError::OnAxis => context.error(SolidCause::OnAxis { sketch }),
        SweepError::Invalid { entities, .. } if !entities.is_empty() => {
            log::warn!("{} could not be built: {error}", context.feature.name);
            context.error_in_sketch(SolidCause::RegionUnbuilt {
                shape,
                sketch,
                curves: context.curves(entities),
                several: entities.len() > 1,
            })
        }
        SweepError::TaperTooSteep => steep_taper(context),
        SweepError::TaperedUnsupportedCurve { entities } => {
            context.error_in_sketch(SolidCause::TaperUnsupported {
                sketch,
                curves: context.curves(entities),
            })
        }
        SweepError::TaperedTiltedEnd => context.error(SolidCause::TaperTilted {
            shape,
            sketch,
            feature: context.feature.name.clone(),
        }),
        SweepError::TaperCloses { entities } => context.error(SolidCause::TaperCloses {
            feature: context.feature.name.clone(),
            sketch,
            at: (!entities.is_empty()).then(|| context.curves(entities)),
            shape,
        }),
        SweepError::Unassembled | SweepError::Geometry(_) | SweepError::Invalid { .. } => {
            log::warn!("{} could not be built: {error}", context.feature.name);
            context.error(SolidCause::Unbuilt { shape, sketch })
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
    let body_name = inputs.document.body_label(body).unwrap_or_default();
    let failure = match error {
        BooleanError::Cancelled(_) => Failure::Cancelled,
        BooleanError::Empty => context.error(SolidCause::Vanishes {
            body: body_name,
            intersecting: matches!(operation, BodyOperation::Intersect(_)),
        }),
        BooleanError::NonManifold(_) => context.error(SolidCause::Edgewise { body: body_name }),
        BooleanError::Intersection { .. }
        | BooleanError::Split(_)
        | BooleanError::Ambiguous(_)
        | BooleanError::Open(_)
        | BooleanError::Invalid(_) => {
            log::warn!("{} could not be combined: {error}", context.feature.name);
            let trouble = boolean_trouble(
                inputs.document,
                operands,
                error,
                Adjust::MoveOrResize {
                    shape: "shape".to_owned(),
                },
            );
            let place = trouble.place;
            context
                .error(SolidCause::Uncombined {
                    body: body_name,
                    trouble,
                })
                .placed(place)
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
    let expected = if dimension == Dimension::ANGLE {
        Expected::Angle(90.0)
    } else if dimension == Dimension::NONE {
        Expected::Number(0.5)
    } else {
        Expected::Length(10.0)
    };
    expression
        .evaluate_as(dimension, &|id| parameters.value(id))
        .map_err(|error| {
            Failure::from(FeatureError::unevaluable(
                context.feature.id(),
                what,
                expected,
                error,
            ))
        })
}

fn positive(context: &Context<'_>, value: f64, what: &str) -> Result<f64, Failure> {
    if value > 0.0 {
        Ok(value)
    } else {
        Err(context.error(SolidCause::NotPositive {
            what: what.to_owned(),
        }))
    }
}

fn within_reach(context: &Context<'_>, value: f64, what: &str) -> Result<f64, Failure> {
    if value <= MAX_SIZE {
        return Ok(value);
    }
    Err(Failure::from(FeatureError::too_big(context.own(), what)))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Side {
    Only { reversed: bool },
    Forward,
    Backward,
}

impl Side {
    fn reached(self) -> ReachedSide {
        match self {
            Self::Only { .. } => ReachedSide::Only,
            Self::Forward => ReachedSide::Forward,
            Self::Backward => ReachedSide::Backward,
        }
    }

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

    fn offset(self) -> &'static str {
        match self {
            Self::Only { .. } => "end offset",
            Self::Forward => "forward end offset",
            Self::Backward => "backward end offset",
        }
    }

    pub(crate) fn ahead(self) -> &'static str {
        match self {
            Self::Only { .. } => "ahead of the sketch",
            Self::Forward => "on the forward side of the sketch",
            Self::Backward => "on the backward side of the sketch",
        }
    }

    pub(crate) fn other_way(self) -> &'static str {
        match self {
            Self::Only { .. } => "Turn Reversed on or off to go the other way",
            Self::Forward | Self::Backward => "Choose another end for that side",
        }
    }
}

fn feature_name(inputs: &Inputs<'_>, feature: FeatureId) -> String {
    inputs.document.body_label(feature).unwrap_or_default()
}

struct Ends<'a> {
    context: &'a Context<'a>,
    inputs: &'a Inputs<'a>,
    operation: BodyOperation,
    other_bodies: &'a [FeatureId],
    plane: Plane,
    regions: &'a [Region],
    stop: Cell<Option<StopAt>>,
    reached: RefCell<Vec<ReachedEnd>>,
    along: Option<Vector3>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct StopAt {
    body: FeatureId,
    far: f64,
    reversed: bool,
    face: Option<FaceName>,
}

impl Ends<'_> {
    fn error(&self, cause: SolidCause) -> Failure {
        self.context.error(cause)
    }

    fn extent(&self, extent: &ExtrudeExtent) -> Result<(LinearExtent, Option<StopAt>), Failure> {
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
                LinearExtent::symmetric(self.length(distance, "distance")? * self.rise())
            }
            ExtrudeExtent::TwoSides { forward, backward } => LinearExtent::between(
                self.bound(backward, Side::Backward)?,
                self.bound(forward, Side::Forward)?,
            ),
        };
        let built = built.map_err(|error| match error {
            SweepError::ZeroLength => self.error(SolidCause::ExtrusionEmpty),
            other => sweep_failure(self.context, "extrusion", &other),
        })?;
        let stop = self.stop.get();
        if stop.is_some() && !matches!(extent, ExtrudeExtent::OneSide { .. }) {
            return Err(self.error(SolidCause::TwoSidedStop {
                sketch: self.context.sketch_name.clone(),
            }));
        }
        Ok((built, stop))
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
                side.sign() * self.length(distance, side.distance())? * self.rise(),
            )),
            ExtrudeEnd::ThroughAll => self.through_all(side),
            ExtrudeEnd::UpToNext { offset } => {
                self.square_only(SquareEnd::UpToNext)?;
                let offset = self.offset(offset.as_deref(), side)?;
                self.up_to_next(side, offset)
            }
            ExtrudeEnd::UpToFace { target, offset } => {
                let offset = self.offset(offset.as_deref(), side)?;
                self.up_to_face(target, side, offset)
            }
            ExtrudeEnd::UpToSurface { face } => {
                self.square_only(SquareEnd::UpToCurved)?;
                self.up_to_surface(face, side)
            }
        }
    }

    fn up_to_surface(&self, face: &FaceAttachment, side: Side) -> Result<LinearBound, Failure> {
        let solid = self.body(face.body)?;
        let body_name = feature_name(self.inputs, face.body);
        let found = face.face.resolve(solid).map_err(|error| {
            self.error(SolidCause::SurfaceFace {
                body: body_name.clone(),
                split: matches!(error, ReferenceError::Ambiguous(_)),
            })
        })?;
        let name = describe_origin(self.inputs.document, face.face.origin());
        if let Some(plane) = face_plane(solid, found) {
            return self.offset_end(plane, side, 0.0, &name);
        }
        let stop = self.stop_at(face.body, side)?;
        if let Some(mut known) = self.stop.get() {
            known.face = solid.face(found).map(|found| found.name());
            self.stop.set(Some(known));
        }
        Ok(stop)
    }

    fn offset(&self, offset: Option<&Expression>, side: Side) -> Result<f64, Failure> {
        let Some(offset) = offset else {
            return Ok(0.0);
        };
        let what = side.offset();
        evaluate_value(
            self.context,
            offset,
            Dimension::LENGTH,
            what,
            self.inputs.parameters,
        )
        .and_then(|value| within_reach(self.context, value.abs(), what).map(|_| value))
    }

    fn offset_end(
        &self,
        target: Plane,
        side: Side,
        offset: f64,
        name: &str,
    ) -> Result<LinearBound, Failure> {
        self.check_ahead(target, side, name)?;
        if offset == 0.0 {
            return Ok(LinearBound::Plane(target));
        }
        let direction = self.plane.normal() * side.sign();
        let onwards = if target.normal().dot(direction) < 0.0 {
            -target.normal()
        } else {
            target.normal()
        };
        let moved = Plane::from_frame(
            target.origin() + onwards * offset,
            target.normal(),
            target.x_axis(),
        )
        .ok_or_else(|| {
            self.error(SolidCause::EndNotMoved {
                name: name.to_owned(),
                offset: side.offset(),
            })
        })?;
        let found = self
            .heights(&moved)
            .map_err(|error| sweep_failure(self.context, "extrusion", &error))?;
        let least = if side.reversed() {
            -found.most
        } else {
            found.least
        };
        if least <= LINEAR_RESOLUTION {
            return Err(self.error(SolidCause::EndOffsetShort {
                offset,
                name: name.to_owned(),
                label: side.offset(),
            }));
        }
        Ok(LinearBound::Plane(moved))
    }

    fn body(&self, body: FeatureId) -> Result<&Solid, Failure> {
        self.inputs
            .body(body)
            .ok_or_else(|| self.inputs.missing_body(body))
    }

    fn through_all(&self, side: Side) -> Result<LinearBound, Failure> {
        let body = match self.operation {
            BodyOperation::Remove(body) | BodyOperation::Intersect(body) => body,
            BodyOperation::NewBody | BodyOperation::Add(_) => {
                return Err(self.error(SolidCause::ThroughAllAdds));
            }
        };
        let mut bounds = self.body(body)?.bounding_box();
        for other in self.other_bodies {
            if let Some(other) = self.body(*other)?.bounding_box() {
                bounds = Some(bounds.map_or(other, |bounds| bounds.union(other)));
            }
        }
        let direction = self.plane.normal() * side.sign();
        let farthest = bounds.map_or(f64::NEG_INFINITY, |bounds| {
            bounds
                .corners()
                .iter()
                .map(|corner| (*corner - self.plane.origin()).dot(direction))
                .fold(f64::NEG_INFINITY, f64::max)
        });
        if farthest <= LINEAR_RESOLUTION {
            return Err(self.error(SolidCause::NothingToGoThrough {
                body: feature_name(self.inputs, body),
                side,
            }));
        }
        let margin =
            bounds.map_or(0.0, |bounds| bounds.diagonal()) * THROUGH_ALL_REACH + THROUGH_ALL_MARGIN;
        Ok(LinearBound::Offset(
            side.sign() * (farthest + margin).min(MAX_SIZE),
        ))
    }

    fn up_to_next(&self, side: Side, offset: f64) -> Result<LinearBound, Failure> {
        let Some(body) = self.operation.target() else {
            return Err(self.error(SolidCause::NextNeedsBody));
        };
        let solid = self.body(body)?;
        let found = match next_face(solid, &self.plane, self.regions, side.reversed()) {
            Ok(found) => found,
            Err(ReachError::Curved(_) | ReachError::SeveralFaces(_)) if offset == 0.0 => {
                return self.stop_at(body, side);
            }
            Err(error @ (ReachError::Curved(_) | ReachError::SeveralFaces(_))) => {
                let found = match self.next_failure(solid, body, side, &error) {
                    Failure::Error(found) => found,
                    other @ (Failure::Waiting(_) | Failure::Cancelled) => return Err(other),
                };
                return Err(self.error(SolidCause::NextUnoffsettable {
                    reached: found.cause,
                }));
            }
            Err(error) => return Err(self.next_failure(solid, body, side, &error)),
        };
        let body_name = feature_name(self.inputs, body);
        let sketch = &self.context.sketch_name;
        match (self.operation, found.entering) {
            (BodyOperation::Add(_), false) => Err(self.error(SolidCause::AddInside {
                sketch: sketch.clone(),
                body: body_name,
            })),
            (BodyOperation::Remove(_), true) => Err(self.error(SolidCause::RemoveEntering {
                sketch: sketch.clone(),
                body: body_name,
            })),
            _ => {
                let face = describe_origin(
                    self.inputs.document,
                    solid.face(found.face).and_then(|face| face.origin()),
                );
                self.reached.borrow_mut().push(ReachedEnd {
                    side: side.reached(),
                    plane: found.plane,
                });
                self.offset_end(found.plane, side, offset, &face)
            }
        }
    }

    fn stop_at(&self, body: FeatureId, side: Side) -> Result<LinearBound, Failure> {
        let bounds = self.body(body)?.bounding_box();
        let direction = self.plane.normal() * side.sign();
        let farthest = bounds.map_or(f64::NEG_INFINITY, |bounds| {
            bounds
                .corners()
                .iter()
                .map(|corner| (*corner - self.plane.origin()).dot(direction))
                .fold(f64::NEG_INFINITY, f64::max)
        });
        let margin =
            bounds.map_or(0.0, |bounds| bounds.diagonal()) * THROUGH_ALL_REACH + THROUGH_ALL_MARGIN;
        let far = (farthest + margin).min(MAX_SIZE);
        self.stop.set(Some(StopAt {
            body,
            far,
            reversed: side.reversed(),
            face: None,
        }));
        Ok(LinearBound::Offset(side.sign() * far))
    }

    fn next_failure(
        &self,
        solid: &Solid,
        body: FeatureId,
        side: Side,
        error: &ReachError,
    ) -> Failure {
        let body = feature_name(self.inputs, body);
        let sketch = self.context.sketch_name.clone();
        let describe = |face: FaceId| {
            describe_origin(
                self.inputs.document,
                solid.face(face).and_then(|face| face.origin()),
            )
        };
        match error {
            ReachError::Cancelled(_) => Failure::Cancelled,
            ReachError::NoRegions => self.context.no_regions(),
            ReachError::Nothing => self.error(SolidCause::ReachNothing { sketch, body, side }),
            ReachError::Partly => self.error(SolidCause::ReachPartly { sketch, body }),
            ReachError::SeveralFaces(faces) => self.error(SolidCause::ReachSeveral {
                sketch,
                body,
                faces: faces.iter().map(|face| describe(*face)).collect(),
            }),
            ReachError::Curved(face) => self.error(SolidCause::ReachCurved {
                face: describe(*face),
            }),
            ReachError::Undecided => self.error(SolidCause::ReachUndecided { body, side }),
        }
    }

    fn up_to_face(
        &self,
        target: &PlaneReference,
        side: Side,
        offset: f64,
    ) -> Result<LinearBound, Failure> {
        let plane = self.resolve(target)?;
        let name = describe_plane(self.inputs.document, target);
        self.offset_end(plane, side, offset, &name)
    }

    fn resolve(&self, target: &PlaneReference) -> Result<Plane, Failure> {
        resolve_target(self.context, self.inputs, target, TargetFor::ExtrusionEnd)
    }

    fn heights(&self, target: &Plane) -> Result<Heights, SweepError> {
        match self.along {
            Some(direction) => heights_along(&self.plane, self.regions, target, direction),
            None => heights(&self.plane, self.regions, target),
        }
    }

    fn rise(&self) -> f64 {
        self.along
            .map_or(1.0, |direction| direction.dot(self.plane.normal()))
    }

    fn square_only(&self, end: SquareEnd) -> Result<(), Failure> {
        if self.along.is_none() {
            return Ok(());
        }
        Err(self.error(SolidCause::SquareOnly { end }))
    }

    fn check_ahead(&self, target: Plane, side: Side, name: &str) -> Result<(), Failure> {
        let found = self.heights(&target).map_err(|error| match error {
            SweepError::EndAlongDirection => self.error(SolidCause::EndAlong {
                name: name.to_owned(),
            }),
            other => sweep_failure(self.context, "extrusion", &other),
        })?;
        let (least, most) = if side.reversed() {
            (-found.most, -found.least)
        } else {
            (found.least, found.most)
        };
        if most <= LINEAR_RESOLUTION {
            return Err(self.error(SolidCause::EndNotAhead {
                name: name.to_owned(),
                side,
            }));
        }
        if least <= LINEAR_RESOLUTION {
            return Err(self.error(SolidCause::EndCuts {
                name: name.to_owned(),
                sketch: self.context.sketch_name.clone(),
            }));
        }
        Ok(())
    }
}

struct Stopping<'a> {
    context: &'a Context<'a>,
    inputs: &'a Inputs<'a>,
    operation: BodyOperation,
    plane: &'a Plane,
    regions: &'a [Region],
}

impl Stopping<'_> {
    fn error(&self, cause: SolidCause) -> Failure {
        self.context.error(cause)
    }

    fn trim(&self, tool: Solid, stop: StopAt) -> Result<Solid, Failure> {
        let body = self
            .inputs
            .body(stop.body)
            .ok_or_else(|| self.inputs.missing_body(stop.body))?;
        let body_name = feature_name(self.inputs, stop.body);
        let sketch = self.context.sketch_name.clone();
        let stopped = stop_at_body(
            &tool,
            body,
            self.plane,
            self.regions,
            stop.reversed,
            stop.far,
        )
        .map_err(|error| match error {
            StopError::Cancelled(_) => Failure::Cancelled,
            StopError::NoRegions => self.context.no_regions(),
            StopError::Straddles => self.error(SolidCause::StopStraddles {
                sketch: sketch.clone(),
                body: body_name.clone(),
            }),
            StopError::Enclosed => self.error(SolidCause::StopEnclosed {
                body: body_name.clone(),
            }),
            StopError::PassesBeside => self.error(SolidCause::ReachPartly {
                sketch: sketch.clone(),
                body: body_name.clone(),
            }),
            StopError::Nothing | StopError::Tessellation(_) => {
                log::warn!(
                    "{} could not stop at the next faces: {error}",
                    self.context.feature.name
                );
                self.error(SolidCause::StopUnbuilt {
                    body: body_name.clone(),
                })
            }
            StopError::Boolean(error) => boolean_failure(
                self.context,
                self.inputs,
                [&tool, body],
                stop.body,
                self.operation,
                &error,
            ),
        })?;
        match (self.operation, stopped.entering) {
            (BodyOperation::Add(target), false) if target == stop.body => {
                return Err(self.error(SolidCause::AddInside {
                    sketch,
                    body: body_name,
                }));
            }
            (BodyOperation::Remove(target), true) if target == stop.body => {
                return Err(self.error(SolidCause::RemoveEntering {
                    sketch,
                    body: body_name,
                }));
            }
            _ => {}
        }
        match stop.face {
            Some(chosen) => self.on_face(body, &body_name, chosen, stopped.solid),
            None => Ok(stopped.solid),
        }
    }
}

impl Stopping<'_> {
    fn on_face(
        &self,
        body: &Solid,
        body_name: &str,
        chosen: FaceName,
        stopped: Solid,
    ) -> Result<Solid, Failure> {
        let of_body: BTreeMap<FaceName, Option<FaceOrigin>> = body
            .faces()
            .map(|(_, face)| (face.name(), face.origin()))
            .collect();
        let mut others: Vec<String> = stopped
            .faces()
            .filter(|(_, face)| face.name() != chosen)
            .filter_map(|(_, face)| of_body.get(&face.name()))
            .map(|origin| describe_origin(self.inputs.document, *origin))
            .collect();
        others.sort();
        others.dedup();
        if others.is_empty() {
            return Ok(stopped);
        }
        let face = of_body
            .get(&chosen)
            .map(|origin| describe_origin(self.inputs.document, *origin))
            .unwrap_or_default();
        Err(self.error(SolidCause::StopBeforeFace {
            sketch: self.context.sketch_name.clone(),
            others,
            body: body_name.to_owned(),
            face,
        }))
    }
}

const HOLDS_PROFILE: f64 = 1e-9;

struct Turn<'a> {
    context: &'a Context<'a>,
    inputs: &'a Inputs<'a>,
    plane: &'a Plane,
    axis: Axis2,
    regions: &'a [Region],
}

impl Turn<'_> {
    fn error(&self, cause: SolidCause) -> Failure {
        self.context.error(cause)
    }

    fn up_to(&self, target: &PlaneReference, reversed: bool) -> Result<AngularExtent, Failure> {
        let found = resolve_target(self.context, self.inputs, target, TargetFor::RevolutionEnd)?;
        let name = capitalized(&describe_plane(self.inputs.document, target));
        let origin = self.plane.to_world(self.axis.origin());
        let direction = self
            .plane
            .to_world(self.axis.origin() + self.axis.direction())
            - origin;
        let through_axis =
            Ray::new(origin, direction).is_some_and(|axis| tolerance::along_plane(axis, &found));
        if !through_axis {
            return Err(self.error(SolidCause::NotThroughAxis { name }));
        }
        let unturnable = || {
            self.error(SolidCause::WouldNotTurn {
                name: name.clone(),
                sketch: self.context.sketch_name.clone(),
            })
        };
        let centre = self
            .regions
            .iter()
            .filter_map(Region::bounds)
            .reduce(Aabb2::union)
            .map(|bounds| self.plane.to_world(bounds.center()))
            .ok_or_else(unturnable)?;
        let axis = direction.normalize();
        let outward = (centre - origin) - axis * (centre - origin).dot(axis);
        let radial = outward.try_normalize().ok_or_else(unturnable)?;
        let normal = found.normal();
        let first = (-normal.dot(radial))
            .atan2(normal.dot(axis.cross(radial)))
            .rem_euclid(PI);
        if first <= HOLDS_PROFILE || PI - first <= HOLDS_PROFILE {
            return Err(unturnable());
        }
        let angle = if reversed { first - PI } else { first };
        AngularExtent::one_side(angle)
            .map_err(|error| sweep_failure(self.context, "revolution", &error))
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
        RevolveExtent::Full | RevolveExtent::UpTo { .. } => Ok(AngularExtent::full()),
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
                SweepError::BeyondFullTurn => context.error(SolidCause::AnglesOverflow),
                other => sweep_failure(context, "revolution", &other),
            });
        }
    };
    built.map_err(|error| sweep_failure(context, "revolution", &error))
}

fn extrusion_direction(
    context: &Context<'_>,
    feature: &Feature,
    inputs: &Inputs<'_>,
    plane: &Plane,
    reference: &AxisReference,
) -> Result<Vector3, Failure> {
    let axis = Resolver { feature, inputs }.axis(reference)?;
    let normal = plane.normal();
    let direction = axis.direction().normalize();
    if tolerance::perpendicular(direction, normal) {
        return Err(context.error(SolidCause::DirectionAlong {
            axis: capitalized(&describe_axis(inputs.document, reference)),
            sketch: context.sketch_name.clone(),
        }));
    }
    Ok(if direction.dot(normal) < 0.0 {
        -direction
    } else {
        direction
    })
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
        _ => Err(context.error(SolidCause::AxisOffPlane {
            axis: capitalized(&describe_axis(inputs.document, reference)),
            sketch: context.sketch_name.clone(),
        })),
    }
}

const REVOLUTION_AXIS_ENTITY: u64 = u64::MAX - 3;

fn one_side(
    context: &Context<'_>,
    sketch: &SketchResult,
    chosen: &[Region],
    axis: Axis2,
    side: AxisSide,
) -> Result<Vec<Region>, Failure> {
    let nothing_left = || {
        context.error(SolidCause::NoSideLeft {
            sketch: context.sketch_name.clone(),
            side,
        })
    };
    let bounds = chosen
        .iter()
        .filter_map(Region::bounds)
        .reduce(Aabb2::union)
        .ok_or_else(nothing_left)?;
    let mut curves = profile_curves(&sketch.geometry);
    curves.push(axis_line(axis, bounds));
    let divided = Profile::new(&curves).map_err(|error| profile_failure(context, &error))?;
    let kept: Vec<_> = divided
        .regions()
        .iter()
        .filter(|region| {
            region.anchor().is_some_and(|anchor| {
                side.holds(axis.signed_distance(anchor))
                    && chosen.iter().any(|whole| whole.contains(anchor))
            })
        })
        .map(Region::key)
        .collect();
    if kept.is_empty() {
        return Err(nothing_left());
    }
    divided
        .select(&Selection::Regions(kept))
        .map_err(|error| profile_failure(context, &error))
}

fn axis_line(axis: Axis2, bounds: Aabb2) -> ProfileCurve {
    let (min, max) = (bounds.min(), bounds.max());
    let reach = bounds.size().length() + 1.0;
    let along = [
        min,
        Point2::new(min.x, max.y),
        Point2::new(max.x, min.y),
        max,
    ]
    .map(|corner| axis.direction().dot(corner - axis.origin()));
    let first = along.into_iter().fold(f64::INFINITY, f64::min) - reach;
    let last = along.into_iter().fold(f64::NEG_INFINITY, f64::max) + reach;
    ProfileCurve::line(
        REVOLUTION_AXIS_ENTITY,
        axis.origin() + axis.direction() * first,
        axis.origin() + axis.direction() * last,
    )
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
        context.error(SolidCause::NotALine {
            axis: sketch.entity_label(axis),
            sketch: context.sketch_name.clone(),
        })
    })
}
