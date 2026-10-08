use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use caditor_document::{
    DatumResult, Document, Evaluation, Feature, FeatureId, FeatureResult, FeatureState,
    PrincipalGeometry, RegionChoice, RevolveAxis, SketchRegion, SolidFeature, SolidResult,
    body_parts, displayed_axis,
};
use caditor_geometry::{Aabb, Plane, Point2, Point3, Ray, RigidTransform};
use caditor_kernel::{RegionKey, RegionMesh, RegionReference, resolve_regions};
use caditor_render::{
    Batch, Color, FaceStyle, Fill, Grid, Layer, Line, Marker, MeshInstance, PickHit, PickId,
    PickResult, Scene, Stroke,
};
use caditor_sketch::{
    Constraint, ConstraintId, Entity, EntityId, EntityState, Faceting, Reference, Sketch,
    SketchSolution,
};

use crate::{
    bodies::{self, BodyBefore, BodyFace, BodyMesh, BodyMeshes, FaceKey, OpenChoice},
    body_appearance, canvas, datum_tools,
    display::DisplayedSketches,
    display_style::DisplayStyle,
    drawing::Preview,
    editing::Context,
    interference_panel::{Mark, MarkKind},
    scene_palette::{Contrast, Highlights, PointFill, ScenePalette, SketchState},
    selection::{self, Axis, Pickable, PrincipalPlane, Selection, SelectionFilter},
    visibility,
};

pub const fn opaque(color: egui::Color32) -> Color {
    Color::from_rgb8(color.r(), color.g(), color.b())
}

const MIN_REFERENCE_SIZE: f64 = 20.0;
const EMPTY_SKETCH_HALF_SIZE: f64 = 50.0;
const BOUNDS_SEGMENT_ANGLE: f64 = std::f64::consts::PI / 60.0;
const SKETCH_SEGMENT_BUDGET: usize = 1 << 19;
const MAX_COARSENINGS: u32 = 64;
const REFERENCE_MARGIN: f64 = 1.2;

const BODY: Color = Color::from_rgb8(
    body_appearance::DEFAULT_COLOUR.red,
    body_appearance::DEFAULT_COLOUR.green,
    body_appearance::DEFAULT_COLOUR.blue,
);
const UNMARKED_VERTEX: Color = Color::from_rgba8(0, 0, 0, 0);
const SNAP_MARKER: Color = opaque(canvas::SNAP);
const MEASURED: Color = opaque(canvas::MEASURE);
const PROBLEM: Color = opaque(canvas::ERROR);
const TOUCH: Color = opaque(canvas::MEASURE);
const UNCHECKED: Color = opaque(canvas::WARNING);
const HIGHLIGHT_FILL_ALPHA: f32 = 0.22;
const HOVERED_REGION_ALPHA: f32 = 0.4;
const DATUM_PLANE_SCALE: f64 = 0.75;
const OPENED_DATUM_EXTRA_WIDTH: f32 = 1.0;

const GUIDE_WIDTH: f32 = 1.0;
const XRAY_FACE_ALPHA: f32 = 0.18;
const PREVIEW_ALPHA: f32 = 0.45;
const GHOST_ALPHA: f32 = 0.2;
const CUT_PREVIEW_ALPHA: f32 = 0.35;
const REVOLVE_AXIS_WIDTH: f32 = 2.5;
const CHOSEN_EDGE_EXTRA_WIDTH: f32 = 1.5;
const AXIS_WIDTH: f32 = 2.0;
const DATUM_POINT_DIAMETER: f32 = 9.0;
const OPENED_DATUM_POINT_EXTRA: f32 = 3.0;
const SKETCH_AXIS_WIDTH: f32 = 1.5;
const PLANE_EDGE_WIDTH: f32 = 1.25;
const HIGHLIGHT_EXTRA_WIDTH: f32 = 1.5;
const ORIGIN_DIAMETER: f32 = 8.0;
const HIGHLIGHT_EXTRA_DIAMETER: f32 = 3.0;
const SNAP_MARKER_DIAMETER: f32 = 13.0;
const MEASURED_WIDTH: f32 = 2.0;
const MEASURED_END_DIAMETER: f32 = 8.0;
const PROBLEM_DIAMETER: f32 = 11.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PickPriority {
    Point,
    Curve,
    Surface,
}

impl PickPriority {
    fn tolerance_points(self) -> f32 {
        match self {
            Self::Point => 7.5,
            Self::Curve => 5.0,
            Self::Surface => 0.5,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PickTable {
    entries: Vec<(Pickable, PickPriority)>,
}

impl PickTable {
    fn register(&mut self, pickable: Pickable, priority: PickPriority) -> Option<PickId> {
        let id = PickId::from_index(self.entries.len())?;
        self.entries.push((pickable, priority));
        Some(id)
    }

    fn resolve(&self, id: PickId) -> Option<(Pickable, PickPriority)> {
        self.entries.get(id.index()).copied()
    }

    #[cfg(test)]
    pub fn id_of(&self, pickable: Pickable) -> Option<PickId> {
        self.entries
            .iter()
            .position(|(candidate, _)| *candidate == pickable)
            .and_then(PickId::from_index)
    }

    pub fn pickables(&self) -> impl Iterator<Item = Pickable> + '_ {
        self.entries.iter().map(|(pickable, _)| *pickable)
    }

    pub fn best_hit(
        &self,
        result: &PickResult,
        filter: SelectionFilter,
    ) -> Option<(Pickable, PickHit)> {
        result
            .hits
            .iter()
            .filter_map(|hit| {
                let (pickable, priority) = self.resolve(hit.id)?;
                (filter.allows(pickable) && hit.offset_points <= priority.tolerance_points())
                    .then_some((priority, pickable, *hit))
            })
            .min_by(|a, b| {
                a.0.cmp(&b.0)
                    .then(a.2.offset_points.total_cmp(&b.2.offset_points))
            })
            .map(|(_, pickable, hit)| (pickable, hit))
    }
}

pub struct Highlight<'a> {
    pub selection: &'a Selection,
    pub hovered: &'a [Pickable],
}

impl Highlight<'_> {
    pub fn is_hovered(&self, pickable: Pickable) -> bool {
        self.hovered.contains(&pickable)
    }

    fn color(&self, highlights: &Highlights, pickable: Pickable, base: Color) -> Color {
        let hovered = self.is_hovered(pickable);
        match (self.selection.contains(pickable), hovered) {
            (true, true) => highlights.hovered_selected,
            (true, false) => highlights.selected,
            (false, true) => highlights.hovered,
            (false, false) => base,
        }
    }

    fn fill_color(&self, highlights: &Highlights, pickable: Pickable, base: Color) -> Color {
        let color = self.color(highlights, pickable, base);
        if color == base {
            base
        } else {
            color.with_alpha(HIGHLIGHT_FILL_ALPHA)
        }
    }

    fn emphasis(&self, pickable: Pickable, selection_widening: f32) -> f32 {
        match (self.selection.contains(pickable), self.is_hovered(pickable)) {
            (true, _) => selection_widening,
            (false, true) => 1.0,
            (false, false) => 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EditedSketch {
    pub feature: FeatureId,
    pub plane: Plane,
    pub bounds: Aabb,
}

#[derive(Debug, Clone)]
pub struct BuiltScene {
    pub scene: Scene,
    pub picks: Arc<PickTable>,
    pub generation: u64,
    pub everything: Aabb,
    pub model: Option<Aabb>,
    pub edited: Option<EditedSketch>,
    reference_size: f64,
}

pub struct Sources<'a> {
    pub document: &'a Document,
    pub evaluation: &'a Evaluation,
    pub bodies: &'a BodyMeshes,
    pub sketches: &'a DisplayedSketches,
    pub style: DisplayStyle,
    pub contrast: Contrast,
}

impl BuiltScene {
    pub fn bounds_of<'a>(
        &self,
        sources: &Sources<'_>,
        pickables: impl IntoIterator<Item = Pickable> + 'a,
    ) -> Option<Aabb> {
        Aabb::from_points(
            pickables
                .into_iter()
                .flat_map(|pickable| pickable_points(sources, pickable, self.reference_size)),
        )
    }

    pub fn fit_all(&self) -> Aabb {
        match self.edited {
            Some(edited) => edited.bounds,
            None => self.model.unwrap_or(self.everything),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Outline {
    Point(Point3),
    Curve(Vec<Segment>),
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Segment {
    start: Point3,
    end: Point3,
    stroke: Stroke,
}

#[derive(Debug, Clone, PartialEq)]
struct SketchShape {
    entity: EntityId,
    state: SketchState,
    outline: Outline,
}

#[derive(Debug, Clone)]
pub struct SketchShapes {
    faceting: Faceting,
    sketches: BTreeMap<FeatureId, Option<Vec<SketchShape>>>,
}

impl SketchShapes {
    pub fn new(faceting: Faceting) -> Self {
        Self {
            faceting,
            sketches: BTreeMap::new(),
        }
    }

    pub fn faceting(&self) -> Faceting {
        self.faceting
    }

    fn of(&mut self, sources: &Sources<'_>, feature: &Feature) -> Option<&[SketchShape]> {
        let faceting = self.faceting;
        self.sketches
            .entry(feature.id())
            .or_insert_with(|| {
                let displayed = sources.sketches.get(sources.evaluation, feature)?;
                let states = ConstraintStates::of(sources.evaluation, feature)?;
                Some(sketch_shapes(&displayed, &states, faceting))
            })
            .as_deref()
    }
}

fn previewed_bodies(evaluation: &Evaluation, context: Context) -> Vec<FeatureId> {
    if context.choosing_in_view || context.sketch.is_some() {
        return Vec::new();
    }
    let Some(result) = context
        .solid
        .and_then(|feature| evaluation.feature(feature)?.result.as_ref())
    else {
        return Vec::new();
    };
    body_parts(result)
        .filter_map(|part| part.solid().map(|solid| solid.body))
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OpenView {
    Before,
    Result,
    Ghost,
}

impl OpenView {
    fn of(evaluation: &Evaluation, bodies: &BodyMeshes, open: &BodyBefore) -> Self {
        let computed = match bodies.draft() {
            Some(draft) => draft.computed,
            None => {
                !evaluation.is_pending(open.feature)
                    && evaluation
                        .feature(open.feature)
                        .is_some_and(|status| matches!(status.state, FeatureState::UpToDate))
            }
        };
        match open.choice {
            OpenChoice::Nothing => Self::Ghost,
            OpenChoice::Edges { .. } if computed => Self::Result,
            OpenChoice::Edges { .. } | OpenChoice::Faces { .. } => Self::Before,
        }
    }
}

fn cutting_feature(evaluation: &Evaluation, context: Context) -> Option<FeatureId> {
    if context.choosing_in_view || context.sketch.is_some() {
        return None;
    }
    context
        .solid
        .filter(|feature| !evaluation.cuts(*feature).is_empty())
}

fn sketch_shapes(
    sketch: &Sketch,
    states: &ConstraintStates<'_>,
    faceting: Faceting,
) -> Vec<SketchShape> {
    let plane = sketch.plane();
    sketch
        .entities()
        .filter_map(|(entity, kind)| {
            let outline = match kind {
                Entity::Point(position) => Outline::Point(plane.to_world(*position)),
                Entity::Line { .. }
                | Entity::Circle { .. }
                | Entity::Arc { .. }
                | Entity::Spline { .. } => Outline::Curve(
                    curve_segments(
                        plane,
                        &sketch.faceted(entity, faceting)?,
                        sketch.is_construction(entity),
                    )
                    .collect(),
                ),
            };
            Some(SketchShape {
                entity,
                state: states.state(entity),
                outline,
            })
        })
        .collect()
}

pub fn build(
    sources: &Sources<'_>,
    highlight: &Highlight<'_>,
    context: Context,
    shapes: &mut SketchShapes,
) -> BuiltScene {
    let Sources {
        document,
        evaluation,
        bodies,
        sketches,
        style,
        contrast,
    } = *sources;
    let palette = contrast.palette();
    let editing = context.sketch;
    let model = model_bounds(sources);
    let reference_size = reference_size(model);
    let edited = editing
        .and_then(|id| document.feature(id))
        .and_then(|feature| Some((feature, sketches.get(evaluation, feature)?)));
    let grid_plane = edited
        .as_ref()
        .map_or(Plane::XY, |(_, displayed)| displayed.plane());
    let mut builder = Builder {
        scene: Batch::default(),
        meshes: Vec::new(),
        translucent_meshes: Vec::new(),
        overlay_meshes: Vec::new(),
        flat_meshes: Vec::new(),
        picks: PickTable::default(),
        highlight,
        style,
        palette,
    };

    match &edited {
        Some((feature, displayed)) => {
            builder.sketch_references(feature.id(), displayed.plane(), reference_size);
        }
        None => {
            for plane in PrincipalPlane::ALL {
                if context.choosing_plane
                    || visibility::is_principal_shown(document, PrincipalGeometry::Plane(plane))
                {
                    builder.principal_plane(plane, reference_size);
                }
            }
            for axis in Axis::ALL {
                if visibility::is_principal_shown(
                    document,
                    PrincipalGeometry::Axis(axis.principal()),
                ) {
                    builder.axis(axis, reference_size);
                }
            }
            if visibility::is_principal_shown(document, PrincipalGeometry::Origin) {
                builder.origin();
            }
            for feature in document.active_features() {
                let opened = context.solid == Some(feature.id());
                if feature.kind.datum().is_some() && (opened || !feature.hidden) {
                    builder.datum(evaluation, feature.id(), opened, reference_size);
                }
            }
        }
    }
    for (feature, presence) in drawn_sketches(document, editing, context.projecting) {
        if let Some(shapes) = shapes.of(sources, feature) {
            builder.sketch(feature.id(), shapes, presence);
        }
    }
    let cutting = cutting_feature(evaluation, context).filter(|_| editing.is_none());
    let open = bodies.body_before().filter(|open| {
        context.solid == Some(open.feature) && editing.is_none() && cutting.is_none()
    });
    let previewed = match cutting {
        Some(_) => Vec::new(),
        None => previewed_bodies(evaluation, context),
    };
    let open_view = open.map(|open| (open, OpenView::of(evaluation, bodies, open)));
    let moved = bodies.moved();
    for (body, mesh) in bodies.iter() {
        if !visibility::is_shown(document, body) {
            continue;
        }
        match open_view {
            Some((open, OpenView::Before)) if open.body == body => continue,
            Some((open, OpenView::Result)) if open.body == body => {
                let shown = bodies.draft().map_or(mesh, |draft| &draft.mesh);
                builder.open_result(document, evaluation, open, shown);
                continue;
            }
            Some(_) | None => {}
        }
        let color = match editing {
            Some(_) => None,
            None => Some(body_color(palette, document, evaluation, body)),
        };
        let opacity = document
            .feature(body)
            .and_then(|feature| feature.appearance.opacity)
            .map(|percent| f32::from(percent) / 100.0);
        let ghosted = matches!(open_view, Some((open, OpenView::Ghost)) if open.body == body);
        let opacity = match previewed.contains(&body) && !ghosted {
            true => Some(opacity.map_or(PREVIEW_ALPHA, |opacity| opacity.min(PREVIEW_ALPHA))),
            false => opacity,
        };
        let faces = match color {
            Some(color) if color == painted(document, body) => {
                face_colours(document, evaluation, body)
            }
            Some(_) | None => BTreeMap::new(),
        };
        let dashed = palette.troubled_edges_dashed
            && editing.is_none()
            && body_health(document, evaluation, body) != Health::Sound;
        builder.body(
            body,
            mesh,
            (color, &faces, dashed),
            opacity,
            editing.is_none() || context.projecting,
            moved
                .filter(|(moved, _)| *moved == body)
                .map(|(_, placement)| placement),
        );
    }
    match open_view {
        Some((open, OpenView::Before)) => builder.open_before(document, evaluation, open, true),
        Some((open, OpenView::Ghost)) => builder.ghost(document, evaluation, open),
        Some((_, OpenView::Result)) | None => {}
    }
    if cutting.is_some() {
        for cut in bodies.cuts() {
            builder.cut_preview(cut);
        }
    }
    if let Some(feature) = context.solid {
        builder.swept(sources, feature, reference_size);
    }
    if let Some((feature, displayed)) = &edited {
        let pickable = context.selecting;
        builder.closed_regions(evaluation, feature.id(), displayed, pickable, Layer::Front);
    }
    if context.picks_shown_regions() {
        for (feature, _) in drawn_sketches(document, editing, context.projecting) {
            if let Some(displayed) = sketches.get(evaluation, feature) {
                builder.closed_regions(evaluation, feature.id(), &displayed, true, Layer::Model);
            }
        }
    }

    let reference = Aabb::from_points(plane_corners(Plane::XY, reference_size))
        .map(|bounds| bounds.including(Point3::Z * reference_size));
    let everything = match (model, reference) {
        (Some(model), Some(reference)) => model.union(reference),
        (Some(bounds), None) | (None, Some(bounds)) => bounds,
        (None, None) => Aabb::from_point(Point3::ZERO),
    };

    BuiltScene {
        scene: Scene {
            meshes: builder.meshes,
            translucent_meshes: builder.translucent_meshes,
            overlay_meshes: builder.overlay_meshes,
            flat_meshes: builder.flat_meshes,
            batches: vec![Arc::new(builder.scene)],
            grid: Some(Grid {
                plane: grid_plane,
                color: palette.grid,
            }),
        },
        picks: Arc::new(builder.picks),
        generation: 0,
        everything,
        model,
        edited: edited.map(|(feature, displayed)| EditedSketch {
            feature: feature.id(),
            plane: displayed.plane(),
            bounds: sketches
                .bounds(evaluation, feature, sketch_points_bounds)
                .unwrap_or_else(|| empty_sketch_bounds(displayed.plane())),
        }),
        reference_size,
    }
}

fn drawn_sketches(
    document: &Document,
    editing: Option<FeatureId>,
    projecting: bool,
) -> impl Iterator<Item = (&Feature, Presence)> {
    document.active_features().filter_map(move |feature| {
        let presence = match editing {
            Some(edited) if edited == feature.id() => Presence::Edited,
            _ if feature.hidden => return None,
            None => Presence::Normal,
            Some(_) if projecting => Presence::Projectable,
            Some(_) => Presence::Background,
        };
        feature
            .kind
            .sketch()
            .is_some()
            .then_some((feature, presence))
    })
}

pub fn drawn_faceting(sources: &Sources<'_>, context: Context, wanted: Faceting) -> Faceting {
    let mut faceting = wanted;
    for _ in 0..MAX_COARSENINGS {
        if sketch_segments_within(sources, context, faceting, SKETCH_SEGMENT_BUDGET) {
            return faceting;
        }
        faceting = Faceting::within(faceting.chord() * 2.0);
    }
    faceting
}

fn sketch_segments_within(
    sources: &Sources<'_>,
    context: Context,
    faceting: Faceting,
    budget: usize,
) -> bool {
    let Sources {
        document,
        evaluation,
        sketches,
        ..
    } = *sources;
    drawn_sketches(document, context.sketch, context.projecting)
        .filter_map(|(feature, _)| sketches.get(evaluation, feature))
        .try_fold(0usize, |total, sketch| {
            sketch
                .entities()
                .filter_map(|(entity, _)| sketch.facet_segments(entity, faceting))
                .try_fold(total, |sum, segments| {
                    sum.checked_add(segments).filter(|sum| *sum <= budget)
                })
        })
        .is_some()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Presence {
    Normal,
    Edited,
    Background,
    Projectable,
}

impl Presence {
    fn layer(self) -> Layer {
        match self {
            Self::Edited => Layer::Front,
            Self::Normal | Self::Background | Self::Projectable => Layer::Model,
        }
    }
}

struct ConstraintStates<'a> {
    solution: Option<&'a SketchSolution>,
    conflicting: BTreeSet<EntityId>,
    redundant: BTreeSet<EntityId>,
    projected: BTreeSet<EntityId>,
    failed: bool,
}

impl<'a> ConstraintStates<'a> {
    fn of(evaluation: &'a Evaluation, feature: &Feature) -> Option<Self> {
        let definition = feature.kind.sketch()?;
        let mut states = Self {
            solution: None,
            conflicting: BTreeSet::new(),
            redundant: BTreeSet::new(),
            projected: definition.projected().collect(),
            failed: false,
        };
        let Some(status) = evaluation.feature(feature.id()) else {
            return Some(states);
        };
        match &status.state {
            FeatureState::Failed(error) if !error.constraints.is_empty() => {
                states.conflicting = entities_of(definition, &error.constraints);
            }
            FeatureState::Failed(_)
            | FeatureState::Outdated
            | FeatureState::Suppressed
            | FeatureState::RolledBack => states.failed = true,
            FeatureState::UpToDate => {
                states.solution = status
                    .result
                    .as_deref()
                    .and_then(FeatureResult::sketch)
                    .map(|result| &result.solution);
                let redundant: Vec<ConstraintId> = states
                    .solution
                    .into_iter()
                    .flat_map(SketchSolution::redundancies)
                    .flat_map(|redundancy| {
                        std::iter::once(redundancy.constraint)
                            .chain(redundancy.duplicates.iter().copied())
                    })
                    .collect();
                states.redundant = entities_of(definition, &redundant);
            }
        }
        Some(states)
    }

    fn state(&self, entity: EntityId) -> SketchState {
        if self.conflicting.contains(&entity) {
            SketchState::Conflicting
        } else if self.failed {
            SketchState::Failed
        } else if self.redundant.contains(&entity) {
            SketchState::Redundant
        } else if self.projected.contains(&entity) {
            SketchState::Projected
        } else if self
            .solution
            .and_then(|solution| solution.entity_state(entity))
            == Some(EntityState::FullyConstrained)
        {
            SketchState::FullyConstrained
        } else {
            SketchState::UnderConstrained
        }
    }
}

fn entities_of(sketch: &Sketch, constraints: &[ConstraintId]) -> BTreeSet<EntityId> {
    constraints
        .iter()
        .filter_map(|constraint| sketch.constraint(*constraint))
        .flat_map(Constraint::entities)
        .collect()
}

struct Builder<'a> {
    scene: Batch,
    meshes: Vec<MeshInstance>,
    translucent_meshes: Vec<MeshInstance>,
    overlay_meshes: Vec<MeshInstance>,
    flat_meshes: Vec<MeshInstance>,
    picks: PickTable,
    highlight: &'a Highlight<'a>,
    style: DisplayStyle,
    palette: &'static ScenePalette,
}

impl Builder<'_> {
    fn emphasis(&self, pickable: Pickable) -> f32 {
        self.highlight
            .emphasis(pickable, self.palette.selection_widening)
    }

    fn principal_plane(&mut self, plane: PrincipalPlane, size: f64) {
        let pickable = Pickable::Plane(plane);
        let pick = self.picks.register(pickable, PickPriority::Surface);
        let corners = plane_corners(plane.plane(), size);
        let edge = self
            .highlight
            .color(&self.palette.lines, pickable, self.palette.plane_edge);
        let width = PLANE_EDGE_WIDTH + self.emphasis(pickable) * HIGHLIGHT_EXTRA_WIDTH;
        for (index, start) in corners.iter().enumerate() {
            let Some(end) = corners.get((index + 1) % corners.len()) else {
                continue;
            };
            self.scene.lines.push(Line {
                start: *start,
                end: *end,
                color: edge,
                width,
                layer: Layer::Reference,
                pick,
                stroke: Stroke::Solid,
            });
        }
        self.scene.fills.push(Fill::convex(
            &corners,
            self.highlight
                .fill_color(&self.palette.lines, pickable, self.palette.plane_fill),
            Layer::Reference,
            pick,
        ));
    }

    fn datum(&mut self, evaluation: &Evaluation, feature: FeatureId, opened: bool, size: f64) {
        let pickable = Pickable::Datum(feature);
        let failed = matches!(
            evaluation.feature(feature).map(|status| &status.state),
            Some(FeatureState::Failed(_) | FeatureState::Outdated)
        );
        let (edge, fill) = if failed {
            (
                self.palette.failed_datum_edge,
                self.palette.failed_datum_fill,
            )
        } else {
            (self.palette.datum_edge, self.palette.datum_fill)
        };
        let extra = if opened {
            OPENED_DATUM_EXTRA_WIDTH
        } else {
            0.0
        };
        let width = PLANE_EDGE_WIDTH + extra + self.emphasis(pickable) * HIGHLIGHT_EXTRA_WIDTH;
        let color = self.highlight.color(&self.palette.lines, pickable, edge);
        let pick = self.picks.register(pickable, PickPriority::Curve);
        let line = |start: Point3, end: Point3, width: f32| Line {
            start,
            end,
            color,
            width,
            layer: Layer::Reference,
            pick,
            stroke: Stroke::Solid,
        };
        match datum_tools::result(evaluation, feature) {
            Some(DatumResult::Plane(plane)) => {
                let corners = datum_plane_corners(plane, size);
                let next = corners.iter().cycle().skip(1);
                let outline: Vec<Line> = corners
                    .iter()
                    .zip(next)
                    .map(|(start, end)| line(*start, *end, width))
                    .collect();
                self.scene.lines.extend(outline);
                self.scene.fills.push(Fill::convex(
                    &corners,
                    self.highlight
                        .fill_color(&self.palette.lines, pickable, fill),
                    Layer::Reference,
                    self.picks.register(pickable, PickPriority::Surface),
                ));
            }
            Some(DatumResult::Axis(ray)) => {
                let [start, end] = axis_ends(ray, Point3::ZERO, size);
                let axis = line(start, end, width + AXIS_WIDTH - PLANE_EDGE_WIDTH);
                self.scene.lines.push(axis);
            }
            Some(DatumResult::Point(position)) => {
                let extra = if opened {
                    OPENED_DATUM_POINT_EXTRA
                } else {
                    0.0
                };
                self.scene.markers.push(Marker {
                    position,
                    color,
                    diameter: DATUM_POINT_DIAMETER
                        + extra
                        + self.emphasis(pickable) * HIGHLIGHT_EXTRA_DIAMETER,
                    layer: Layer::Reference,
                    pick: self.picks.register(pickable, PickPriority::Point),
                });
            }
            None => {}
        }
    }

    fn body(
        &mut self,
        body: FeatureId,
        mesh: &BodyMesh,
        (color, face_colours, dashed): (Option<Color>, &BTreeMap<FaceKey, Color>, bool),
        opacity: Option<f32>,
        pickable: bool,
        placement: Option<RigidTransform>,
    ) {
        let placed =
            |point: Point3| placement.map_or(point, |placement| placement.apply_point(point));
        let own =
            |face: &BodyFace, base: Color| face_colours.get(&face.key).copied().unwrap_or(base);
        let style = match color {
            Some(_) => self.style,
            None => DisplayStyle::default(),
        };
        let see_through = match (style.is_translucent(), opacity) {
            (true, Some(opacity)) => Some(opacity.min(XRAY_FACE_ALPHA)),
            (true, None) => Some(XRAY_FACE_ALPHA),
            (false, opacity) if style.shows_faces() => opacity,
            (false, _) => None,
        };
        if let (Some(alpha), Some(base)) = (see_through, color) {
            let picked = pickable && !style.is_translucent();
            let faces = mesh
                .faces
                .iter()
                .map(|face| {
                    let pickable = Pickable::Face {
                        body,
                        face: face.key,
                    };
                    let base = own(face, base);
                    match picked {
                        true => FaceStyle {
                            color: self
                                .highlight
                                .color(&self.palette.faces, pickable, base)
                                .with_alpha(alpha),
                            pick: self.picks.register(pickable, PickPriority::Surface),
                        },
                        false => FaceStyle {
                            color: base.with_alpha(alpha),
                            pick: None,
                        },
                    }
                })
                .collect();
            self.translucent_meshes.push(MeshInstance {
                mesh: Arc::clone(&mesh.mesh),
                faces,
                placement,
            });
        } else if style.shows_faces() {
            let face_base = |base: Color| {
                if style.is_drawing() {
                    self.palette.drawing_face
                } else {
                    base
                }
            };
            let faces = mesh
                .faces
                .iter()
                .map(|face| match color {
                    Some(base) => {
                        let base = face_base(own(face, base));
                        let pickable = Pickable::Face {
                            body,
                            face: face.key,
                        };
                        FaceStyle {
                            color: self.highlight.color(&self.palette.faces, pickable, base),
                            pick: self.picks.register(pickable, PickPriority::Surface),
                        }
                    }
                    None if pickable => {
                        let pickable = Pickable::Face {
                            body,
                            face: face.key,
                        };
                        FaceStyle {
                            color: self.highlight.color(
                                &self.palette.faces,
                                pickable,
                                self.palette.background_body,
                            ),
                            pick: self.picks.register(pickable, PickPriority::Surface),
                        }
                    }
                    None => FaceStyle {
                        color: self.palette.background_body,
                        pick: None,
                    },
                })
                .collect();
            let instance = MeshInstance {
                mesh: Arc::clone(&mesh.mesh),
                faces,
                placement,
            };
            if style.is_drawing() {
                self.flat_meshes.push(instance);
            } else {
                self.meshes.push(instance);
            }
        }
        let edge_color = if style.shows_edges() {
            self.palette.body_edge
        } else {
            self.palette.body_edge.with_alpha(0.0)
        };
        let edge_width = self.palette.body_edge_width;
        let pickable_edges = pickable;
        let shows_hidden_edges = style.shows_hidden_edges() && color.is_some();
        for edge in &mesh.edges {
            let pickable = Pickable::Edge {
                body,
                edge: edge.name,
            };
            let (color, width, pick) = match color {
                Some(_) => (
                    self.highlight
                        .color(&self.palette.lines, pickable, edge_color),
                    edge_width + self.emphasis(pickable) * HIGHLIGHT_EXTRA_WIDTH,
                    self.picks.register(pickable, PickPriority::Curve),
                ),
                None if pickable_edges => (
                    self.highlight.color(
                        &self.palette.lines,
                        pickable,
                        self.palette.background_body_edge,
                    ),
                    edge_width + self.emphasis(pickable) * HIGHLIGHT_EXTRA_WIDTH,
                    self.picks.register(pickable, PickPriority::Curve),
                ),
                None => (self.palette.background_body_edge, edge_width, None),
            };
            let drawn = EdgeStroke {
                color,
                width,
                layer: Layer::Model,
                pick,
                dashed,
            };
            self.scene
                .lines
                .extend(edge_lines(&edge.points, &placed, drawn));
            if shows_hidden_edges && color.alpha > 0.0 {
                let hidden = EdgeStroke {
                    layer: Layer::Hidden,
                    pick: None,
                    dashed: true,
                    ..drawn
                };
                self.scene
                    .lines
                    .extend(edge_lines(&edge.points, &placed, hidden));
            }
        }
        if color.is_none() && !pickable_edges {
            return;
        }
        for vertex in &mesh.vertices {
            let pickable = Pickable::Vertex {
                body,
                vertex: vertex.key,
            };
            self.scene.markers.push(Marker {
                position: placed(vertex.position),
                color: self
                    .highlight
                    .color(&self.palette.lines, pickable, UNMARKED_VERTEX),
                diameter: self.palette.point_diameter
                    + self.emphasis(pickable) * HIGHLIGHT_EXTRA_DIAMETER,
                layer: Layer::Model,
                pick: self.picks.register(pickable, PickPriority::Point),
            });
        }
    }

    fn cut_preview(&mut self, cut: &BodyMesh) {
        self.overlay_meshes.push(MeshInstance {
            mesh: Arc::clone(&cut.mesh),
            faces: vec![
                FaceStyle {
                    color: self.palette.cut_preview.with_alpha(CUT_PREVIEW_ALPHA),
                    pick: None,
                };
                cut.faces.len()
            ],
            placement: None,
        });
        for edge in &cut.edges {
            let segments = edge.points.windows(2).filter_map(|pair| match pair {
                [start, end] => Some(Line {
                    start: *start,
                    end: *end,
                    color: self.palette.cut_preview_edge,
                    width: self.palette.body_edge_width,
                    layer: Layer::Front,
                    pick: None,
                    stroke: Stroke::Solid,
                }),
                _ => None,
            });
            self.scene.lines.extend(segments);
        }
    }

    fn open_result(
        &mut self,
        document: &Document,
        evaluation: &Evaluation,
        open: &BodyBefore,
        mesh: &BodyMesh,
    ) {
        let color = body_color(self.palette, document, evaluation, open.body);
        self.meshes.push(MeshInstance {
            mesh: Arc::clone(&mesh.mesh),
            faces: mesh
                .faces
                .iter()
                .map(|_| FaceStyle { color, pick: None })
                .collect(),
            placement: None,
        });
        for edge in &mesh.edges {
            self.scene
                .lines
                .extend(edge.points.windows(2).filter_map(|pair| match pair {
                    [start, end] => Some(Line {
                        start: *start,
                        end: *end,
                        color: self.palette.body_edge,
                        width: self.palette.body_edge_width,
                        layer: Layer::Model,
                        pick: None,
                        stroke: Stroke::Solid,
                    }),
                    _ => None,
                }));
        }
        self.open_before(document, evaluation, open, false);
    }

    fn ghost(&mut self, document: &Document, evaluation: &Evaluation, open: &BodyBefore) {
        let color =
            body_color(self.palette, document, evaluation, open.body).with_alpha(GHOST_ALPHA);
        self.translucent_meshes.push(MeshInstance {
            mesh: Arc::clone(&open.before.mesh),
            faces: open
                .before
                .faces
                .iter()
                .map(|_| FaceStyle { color, pick: None })
                .collect(),
            placement: None,
        });
    }

    fn open_before(
        &mut self,
        document: &Document,
        evaluation: &Evaluation,
        open: &BodyBefore,
        with_faces: bool,
    ) {
        let (chosen, opened) = match &open.choice {
            OpenChoice::Edges { chosen, .. } => (Some(chosen), None),
            OpenChoice::Faces { opened, .. } => (None, Some(opened)),
            OpenChoice::Nothing => (None, None),
        };
        let color = body_color(self.palette, document, evaluation, open.body);
        let faces = open
            .before
            .faces
            .iter()
            .map(|face| match &opened {
                Some(opened) if face.flat => {
                    let pickable = Pickable::ShellFace {
                        feature: open.feature,
                        face: face.key,
                    };
                    let base = if opened.contains(&face.key) {
                        self.palette.faces.selected
                    } else {
                        color
                    };
                    let color = if self.highlight.is_hovered(pickable) {
                        self.palette.faces.hovered
                    } else {
                        base
                    };
                    FaceStyle {
                        color,
                        pick: self.picks.register(pickable, PickPriority::Surface),
                    }
                }
                Some(_) | None => FaceStyle { color, pick: None },
            })
            .collect();
        if with_faces {
            self.meshes.push(MeshInstance {
                mesh: Arc::clone(&open.before.mesh),
                faces,
                placement: None,
            });
        }
        for edge in &open.before.edges {
            let (color, width, pick) = match &chosen {
                Some(chosen) => {
                    let pickable = Pickable::BlendEdge {
                        feature: open.feature,
                        edge: edge.name,
                    };
                    let (base, extra) = if chosen.explicit.contains(&edge.name) {
                        (self.palette.lines.selected, CHOSEN_EDGE_EXTRA_WIDTH)
                    } else if chosen.followed.contains(&edge.name) {
                        (self.palette.followed_edge, CHOSEN_EDGE_EXTRA_WIDTH)
                    } else {
                        (self.palette.body_edge, 0.0)
                    };
                    let color = if self.highlight.is_hovered(pickable) {
                        self.palette.lines.hovered
                    } else {
                        base
                    };
                    let width = self.palette.body_edge_width
                        + extra
                        + self.emphasis(pickable) * HIGHLIGHT_EXTRA_WIDTH;
                    (
                        color,
                        width,
                        self.picks.register(pickable, PickPriority::Curve),
                    )
                }
                None => (self.palette.body_edge, self.palette.body_edge_width, None),
            };
            let segments = edge.points.windows(2).filter_map(|pair| match pair {
                [start, end] => Some(Line {
                    start: *start,
                    end: *end,
                    color,
                    width,
                    layer: Layer::Model,
                    pick,
                    stroke: Stroke::Solid,
                }),
                _ => None,
            });
            self.scene.lines.extend(segments);
        }
    }

    fn pattern_axes(&mut self, sources: &Sources<'_>, feature: FeatureId, reference_size: f64) {
        let Sources {
            document,
            evaluation,
            ..
        } = *sources;
        let Some(pattern) = document
            .feature(feature)
            .and_then(|owner| owner.kind.pattern())
        else {
            return;
        };
        let near = evaluation
            .body_result(pattern.body)
            .and_then(|result| result.solid())
            .and_then(SolidResult::bounding_box)
            .map_or(Point3::ZERO, |bounds| bounds.center());
        for axis in pattern.axes() {
            let Some(ray) = displayed_axis(evaluation, feature, axis) else {
                continue;
            };
            let [start, end] = axis_ends(ray, near, reference_size);
            self.scene.lines.push(Line {
                start,
                end,
                color: self.palette.revolve_axis,
                width: REVOLVE_AXIS_WIDTH,
                layer: Layer::Model,
                pick: None,
                stroke: Stroke::Solid,
            });
        }
    }

    fn swept(&mut self, sources: &Sources<'_>, feature: FeatureId, reference_size: f64) {
        self.pattern_axes(sources, feature, reference_size);
        let Sources {
            document,
            evaluation,
            sketches,
            ..
        } = *sources;
        let Some(solid) = document
            .feature(feature)
            .and_then(|owner| owner.kind.solid())
        else {
            return;
        };
        let Some(plane) = sketch_plane(document, evaluation, solid.sketch()) else {
            return;
        };
        if let SolidFeature::Revolve(revolve) = solid {
            let displayed = document
                .feature(solid.sketch())
                .and_then(|sketch| sketches.get(evaluation, sketch));
            let ends = match &revolve.axis {
                RevolveAxis::Sketch(line) => match line.reference() {
                    Some(reference) => Some(reference_points(plane, reference, reference_size)),
                    None => displayed
                        .as_deref()
                        .and_then(|sketch| sketch.line_endpoints(*line))
                        .map(|(start, end)| [plane.to_world(start), plane.to_world(end)]),
                },
                RevolveAxis::Model(axis) => displayed_axis(evaluation, feature, axis)
                    .map(|ray| axis_ends(ray, plane.origin(), reference_size)),
            };
            if let Some([start, end]) = ends {
                self.scene.lines.push(Line {
                    start,
                    end,
                    color: self.palette.revolve_axis,
                    width: REVOLVE_AXIS_WIDTH,
                    layer: Layer::Model,
                    pick: None,
                    stroke: Stroke::Solid,
                });
            }
        }
        let Some((_, regions)) = selection::swept_regions(document, evaluation, feature) else {
            return;
        };
        let chosen = chosen_regions(solid.regions(), regions);
        for region in regions {
            let Some(mesh) = &region.mesh else {
                continue;
            };
            let key = region.region.key();
            let pickable = Pickable::Region {
                feature,
                region: key,
            };
            let base = if chosen.contains(&key) {
                self.palette.chosen_region
            } else {
                self.palette.open_region
            };
            self.scene.fills.push(Fill {
                triangles: region_triangles(mesh, &plane),
                color: self.region_color(pickable, base),
                layer: Layer::Front,
                pick: self.picks.register(pickable, PickPriority::Surface),
            });
        }
    }

    fn region_color(&self, pickable: Pickable, base: Color) -> Color {
        if self.highlight.is_hovered(pickable) {
            self.palette.lines.hovered.with_alpha(HOVERED_REGION_ALPHA)
        } else if self.highlight.selection.contains(pickable) {
            self.palette.chosen_region
        } else {
            base
        }
    }

    fn closed_regions(
        &mut self,
        evaluation: &Evaluation,
        feature: FeatureId,
        displayed: &Sketch,
        pickable: bool,
        layer: Layer,
    ) {
        let Some(result) = evaluation
            .feature(feature)
            .and_then(|status| status.result.as_deref())
            .and_then(FeatureResult::sketch)
        else {
            return;
        };
        let Some(Ok(regions)) = result.regions() else {
            return;
        };
        if !result.geometry.same_geometry(displayed) {
            return;
        }
        let plane = displayed.plane();
        for region in regions {
            let Some(mesh) = &region.mesh else {
                continue;
            };
            let region = Pickable::SketchRegion {
                feature,
                region: region.region.key(),
            };
            let pick = match pickable {
                true => self.picks.register(region, PickPriority::Surface),
                false => None,
            };
            self.scene.fills.push(Fill {
                triangles: region_triangles(mesh, &plane),
                color: self.region_color(region, self.palette.closed_region),
                layer,
                pick,
            });
        }
    }

    fn axis(&mut self, axis: Axis, size: f64) {
        let pickable = Pickable::Axis(axis);
        let base = self.palette.axis(axis);
        self.scene.lines.push(Line {
            start: Point3::ZERO,
            end: axis.direction() * size,
            color: self.highlight.color(&self.palette.lines, pickable, base),
            width: AXIS_WIDTH + self.emphasis(pickable) * HIGHLIGHT_EXTRA_WIDTH,
            layer: Layer::Reference,
            pick: self.picks.register(pickable, PickPriority::Curve),
            stroke: Stroke::Solid,
        });
    }

    fn origin(&mut self) {
        let pickable = Pickable::Origin;
        self.scene.markers.push(Marker {
            position: Point3::ZERO,
            color: self
                .highlight
                .color(&self.palette.lines, pickable, self.palette.origin),
            diameter: ORIGIN_DIAMETER + self.emphasis(pickable) * HIGHLIGHT_EXTRA_DIAMETER,
            layer: Layer::Reference,
            pick: self.picks.register(pickable, PickPriority::Point),
        });
    }

    fn sketch_references(&mut self, feature: FeatureId, plane: Plane, size: f64) {
        for (reference, color) in [
            (
                Reference::HorizontalAxis,
                self.palette.sketch_horizontal_axis,
            ),
            (Reference::VerticalAxis, self.palette.sketch_vertical_axis),
        ] {
            let pickable = Pickable::SketchEntity {
                feature,
                entity: reference.id(),
            };
            let [start, end] = reference_points(plane, reference, size);
            self.scene.lines.push(Line {
                start,
                end,
                color: self.highlight.color(&self.palette.lines, pickable, color),
                width: SKETCH_AXIS_WIDTH + self.emphasis(pickable) * HIGHLIGHT_EXTRA_WIDTH,
                layer: Layer::Front,
                pick: self.picks.register(pickable, PickPriority::Curve),
                stroke: Stroke::Solid,
            });
        }
        let pickable = Pickable::SketchEntity {
            feature,
            entity: EntityId::ORIGIN,
        };
        self.scene.markers.push(Marker {
            position: plane.origin(),
            color: self
                .highlight
                .color(&self.palette.lines, pickable, self.palette.origin),
            diameter: ORIGIN_DIAMETER + self.emphasis(pickable) * HIGHLIGHT_EXTRA_DIAMETER,
            layer: Layer::Front,
            pick: self.picks.register(pickable, PickPriority::Point),
        });
    }

    fn sketch(&mut self, feature: FeatureId, shapes: &[SketchShape], presence: Presence) {
        let layer = presence.layer();
        for shape in shapes {
            let pickable = Pickable::SketchEntity {
                feature,
                entity: shape.entity,
            };
            let (state, emphasis, pickable) = match presence {
                Presence::Background => (SketchState::Background, 0.0, None),
                Presence::Projectable => (
                    SketchState::Background,
                    self.emphasis(pickable),
                    (!shape.entity.is_reference()).then_some(pickable),
                ),
                Presence::Normal | Presence::Edited => {
                    (shape.state, self.emphasis(pickable), Some(pickable))
                }
            };
            let look = self.palette.look(state);
            let color = |base: Color| match pickable {
                Some(pickable) => self.highlight.color(&self.palette.lines, pickable, base),
                None => base,
            };
            match &shape.outline {
                Outline::Point(position) => {
                    let color = color(look.point);
                    self.scene.markers.push(Marker {
                        position: *position,
                        color,
                        diameter: self.palette.point_diameter + emphasis * HIGHLIGHT_EXTRA_DIAMETER,
                        layer,
                        pick: pickable.and_then(|pickable| {
                            self.picks.register(pickable, PickPriority::Point)
                        }),
                    });
                    if look.fill == PointFill::Hollow {
                        self.scene.markers.push(Marker {
                            position: *position,
                            color: self.palette.hole,
                            diameter: self.palette.hole_diameter,
                            layer,
                            pick: None,
                        });
                    }
                }
                Outline::Curve(segments) => {
                    let color = color(look.curve);
                    let width =
                        self.palette.curve_width(look.weight) + emphasis * HIGHLIGHT_EXTRA_WIDTH;
                    let pick = pickable
                        .and_then(|pickable| self.picks.register(pickable, PickPriority::Curve));
                    self.scene.lines.extend(segments.iter().map(|segment| Line {
                        start: segment.start,
                        end: segment.end,
                        color,
                        width,
                        layer,
                        pick,
                        stroke: segment.stroke,
                    }));
                }
            }
        }
    }
}

struct CurveStyle {
    color: Color,
    width: f32,
    layer: Layer,
    pick: Option<PickId>,
    dashed: bool,
}

#[derive(Clone, Copy)]
struct EdgeStroke {
    color: Color,
    width: f32,
    layer: Layer,
    pick: Option<PickId>,
    dashed: bool,
}

fn edge_lines(
    points: &[Point3],
    placed: &impl Fn(Point3) -> Point3,
    stroke: EdgeStroke,
) -> Vec<Line> {
    let mut along = 0.0;
    points
        .windows(2)
        .filter_map(|pair| match pair {
            [start, end] => {
                let line_stroke = if stroke.dashed {
                    Stroke::Dashed {
                        along: along as f32,
                    }
                } else {
                    Stroke::Solid
                };
                along += start.distance(*end);
                Some(Line {
                    start: placed(*start),
                    end: placed(*end),
                    color: stroke.color,
                    width: stroke.width,
                    layer: stroke.layer,
                    pick: stroke.pick,
                    stroke: line_stroke,
                })
            }
            _ => None,
        })
        .collect()
}

fn curve_segments(plane: Plane, points: &[Point2], dashed: bool) -> impl Iterator<Item = Segment> {
    let mut along = 0.0;
    points.windows(2).filter_map(move |pair| match *pair {
        [start, end] => {
            let stroke = if dashed {
                Stroke::Dashed {
                    along: along as f32,
                }
            } else {
                Stroke::Solid
            };
            along += start.distance(end);
            Some(Segment {
                start: plane.to_world(start),
                end: plane.to_world(end),
                stroke,
            })
        }
        _ => None,
    })
}

fn curve_lines(plane: Plane, points: &[Point2], style: CurveStyle) -> impl Iterator<Item = Line> {
    curve_segments(plane, points, style.dashed).map(move |segment| Line {
        start: segment.start,
        end: segment.end,
        color: style.color,
        width: style.width,
        layer: style.layer,
        pick: style.pick,
        stroke: segment.stroke,
    })
}

pub fn add_preview(scene: &mut Batch, palette: &ScenePalette, plane: Plane, preview: &Preview) {
    for curve in &preview.curves {
        let style = CurveStyle {
            color: palette.preview_curve,
            width: palette.curve_width,
            layer: Layer::Front,
            pick: None,
            dashed: preview.construction,
        };
        scene.lines.extend(curve_lines(plane, curve, style));
    }
    for curve in &preview.removed {
        let style = CurveStyle {
            color: palette.trimmed_curve,
            width: palette.curve_width + HIGHLIGHT_EXTRA_WIDTH,
            layer: Layer::Front,
            pick: None,
            dashed: false,
        };
        scene.lines.extend(curve_lines(plane, curve, style));
    }
    for guide in &preview.guides {
        let style = CurveStyle {
            color: SNAP_MARKER,
            width: GUIDE_WIDTH,
            layer: Layer::Front,
            pick: None,
            dashed: true,
        };
        scene.lines.extend(curve_lines(plane, guide, style));
    }
    let snap = preview.snap.map(|position| Marker {
        position: plane.to_world(position),
        color: SNAP_MARKER,
        diameter: SNAP_MARKER_DIAMETER,
        layer: Layer::Front,
        pick: None,
    });
    let points = preview.points.iter().map(|position| Marker {
        position: plane.to_world(*position),
        color: palette.preview_point,
        diameter: palette.point_diameter,
        layer: Layer::Front,
        pick: None,
    });
    scene.markers.extend(snap.into_iter().chain(points));
}

pub fn add_measurement(scene: &mut Batch, from: Point3, to: Point3) {
    scene.lines.push(Line {
        start: from,
        end: to,
        color: MEASURED,
        width: MEASURED_WIDTH,
        layer: Layer::Front,
        pick: None,
        stroke: Stroke::Solid,
    });
    for position in [from, to] {
        scene.markers.push(Marker {
            position,
            color: MEASURED,
            diameter: MEASURED_END_DIAMETER,
            layer: Layer::Front,
            pick: None,
        });
    }
}

pub fn add_problem(scene: &mut Batch, position: Point3) {
    scene.markers.push(Marker {
        position,
        color: PROBLEM,
        diameter: PROBLEM_DIAMETER,
        layer: Layer::Front,
        pick: None,
    });
}

pub fn add_interference(scene: &mut Batch, mark: &Mark) {
    let color = match mark.kind {
        MarkKind::Overlap => PROBLEM,
        MarkKind::Touch => TOUCH,
        MarkKind::Unchecked => UNCHECKED,
    };
    scene.markers.push(Marker {
        position: mark.place,
        color,
        diameter: PROBLEM_DIAMETER,
        layer: Layer::Front,
        pick: None,
    });
    let segments = mark
        .outline
        .iter()
        .flat_map(|outline| outline.iter())
        .flat_map(|polyline| polyline.windows(2))
        .filter_map(|pair| match pair {
            [start, end] => Some((*start, *end)),
            _ => None,
        });
    for (start, end) in segments {
        scene.lines.push(Line {
            start,
            end,
            color,
            width: MEASURED_WIDTH,
            layer: Layer::Front,
            pick: None,
            stroke: Stroke::Solid,
        });
    }
}

pub fn chosen_regions(choice: &RegionChoice, regions: &[SketchRegion]) -> BTreeSet<RegionKey> {
    match choice {
        RegionChoice::All => regions
            .iter()
            .filter(|region| region.even_depth)
            .map(|region| region.region.key())
            .collect(),
        RegionChoice::Chosen(references) => {
            match resolve_regions(references, regions.iter().map(|region| &region.region)) {
                Ok(resolved) => resolved.keys.into_iter().collect(),
                Err(_) => references.iter().map(RegionReference::key).collect(),
            }
        }
    }
}

pub fn region_references(
    keys: &BTreeSet<RegionKey>,
    regions: &[SketchRegion],
) -> Vec<RegionReference> {
    regions
        .iter()
        .filter(|region| keys.contains(&region.region.key()))
        .map(SketchRegion::reference)
        .collect()
}

fn face_colours(
    document: &Document,
    evaluation: &Evaluation,
    body: FeatureId,
) -> BTreeMap<FaceKey, Color> {
    let Some(appearance) = document
        .feature(body)
        .map(|feature| &feature.appearance)
        .filter(|appearance| !appearance.faces.is_empty())
    else {
        return BTreeMap::new();
    };
    let Some(shown) = bodies::shown(evaluation, body) else {
        return BTreeMap::new();
    };
    let coloured = appearance.face_colours(&shown.solid);
    bodies::face_keys(&shown.solid)
        .into_iter()
        .filter_map(|(id, key)| {
            let colour = coloured.get(&id)?;
            Some((key, Color::from_rgb8(colour.red, colour.green, colour.blue)))
        })
        .collect()
}

fn painted(document: &Document, body: FeatureId) -> Color {
    document
        .feature(body)
        .and_then(|feature| feature.appearance.colour)
        .map_or(BODY, |colour| {
            Color::from_rgb8(colour.red, colour.green, colour.blue)
        })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Health {
    Sound,
    Outdated,
    Failed,
}

fn body_health(document: &Document, evaluation: &Evaluation, body: FeatureId) -> Health {
    let mut health = Health::Sound;
    for feature in document
        .features()
        .filter(|feature| feature.body() == Some(body))
    {
        match evaluation.feature(feature.id()).map(|status| &status.state) {
            Some(FeatureState::Failed(_)) => return Health::Failed,
            Some(FeatureState::Outdated) => health = Health::Outdated,
            Some(FeatureState::UpToDate | FeatureState::Suppressed | FeatureState::RolledBack)
            | None => {}
        }
    }
    health
}

fn body_color(
    palette: &ScenePalette,
    document: &Document,
    evaluation: &Evaluation,
    body: FeatureId,
) -> Color {
    match body_health(document, evaluation, body) {
        Health::Sound => painted(document, body),
        Health::Outdated => palette.outdated_body,
        Health::Failed => palette.failed_body,
    }
}

fn reference_points(plane: Plane, reference: Reference, size: f64) -> [Point3; 2] {
    let (start, end) = match reference {
        Reference::Origin => (Point2::ZERO, Point2::ZERO),
        Reference::HorizontalAxis => (Point2::new(-size, 0.0), Point2::new(size, 0.0)),
        Reference::VerticalAxis => (Point2::new(0.0, -size), Point2::new(0.0, size)),
    };
    [plane.to_world(start), plane.to_world(end)]
}

fn sketch_entity_points(sketch: &Sketch, entity: EntityId, reference_size: f64) -> Vec<Point3> {
    let plane = sketch.plane();
    if let Some(reference) = entity.reference() {
        return reference_points(plane, reference, reference_size).to_vec();
    }
    match sketch.entity(entity) {
        Some(Entity::Point(position)) => vec![plane.to_world(*position)],
        Some(_) => sketch
            .polyline(entity, BOUNDS_SEGMENT_ANGLE)
            .unwrap_or_default()
            .into_iter()
            .map(|point| plane.to_world(point))
            .collect(),
        None => Vec::new(),
    }
}

fn sketch_points_bounds(sketch: &Sketch) -> Option<Aabb> {
    Aabb::from_points(
        sketch
            .entities()
            .flat_map(|(entity, _)| sketch_entity_points(sketch, entity, 0.0)),
    )
}

fn empty_sketch_bounds(plane: Plane) -> Aabb {
    Aabb::from_points(plane_corners(plane, EMPTY_SKETCH_HALF_SIZE))
        .unwrap_or_else(|| Aabb::from_point(plane.origin()))
}

pub fn sketch_plane(
    document: &Document,
    evaluation: &Evaluation,
    feature: FeatureId,
) -> Option<Plane> {
    let solved = evaluation
        .feature(feature)
        .and_then(|status| status.result.as_deref())
        .and_then(FeatureResult::sketch)
        .map(|result| result.geometry.plane());
    solved.or_else(|| Some(document.feature(feature)?.kind.sketch()?.plane()))
}

fn pickable_points(sources: &Sources<'_>, pickable: Pickable, reference_size: f64) -> Vec<Point3> {
    let Sources {
        document,
        evaluation,
        bodies,
        sketches,
        ..
    } = *sources;
    match pickable {
        Pickable::Origin => vec![Point3::ZERO],
        Pickable::Axis(axis) => vec![Point3::ZERO, axis.direction() * reference_size],
        Pickable::Plane(plane) => plane_corners(plane.plane(), reference_size).to_vec(),
        Pickable::SketchEntity { feature, entity } => document
            .feature(feature)
            .and_then(|owner| sketches.get(evaluation, owner))
            .map(|sketch| sketch_entity_points(&sketch, entity, reference_size))
            .unwrap_or_default(),
        Pickable::SketchConstraint { .. } => pickable
            .constrained_entities(document)
            .into_iter()
            .filter(|entity| {
                !matches!(entity, Pickable::SketchEntity { entity, .. } if entity.is_reference())
            })
            .flat_map(|entity| pickable_points(sources, entity, reference_size))
            .collect(),
        Pickable::Face { body, face } => bodies
            .get(body)
            .and_then(|mesh| mesh.face_bounds(face))
            .map(|bounds| bounds.corners().to_vec())
            .unwrap_or_default(),
        Pickable::Edge { body, edge } => bodies
            .get(body)
            .and_then(|mesh| mesh.edge_points(edge))
            .map(<[Point3]>::to_vec)
            .unwrap_or_default(),
        Pickable::Vertex { body, vertex } => bodies
            .get(body)
            .and_then(|mesh| mesh.vertex_position(vertex))
            .into_iter()
            .collect(),
        Pickable::Datum(feature) => datum_points(evaluation, feature, reference_size),
        Pickable::ShellFace { feature, face } => bodies
            .body_before()
            .filter(|open| open.feature == feature)
            .and_then(|open| open.before.face_bounds(face))
            .map(|bounds| bounds.corners().to_vec())
            .unwrap_or_default(),
        Pickable::BlendEdge { feature, edge } => bodies
            .body_before()
            .filter(|open| open.feature == feature)
            .and_then(|open| open.before.edge_points(edge))
            .map(<[Point3]>::to_vec)
            .unwrap_or_default(),
        Pickable::Region { feature, region } => selection::swept_regions(document, evaluation, feature)
            .map(|(sketch, regions)| region_points(document, evaluation, sketch, regions, region))
            .unwrap_or_default(),
        Pickable::SketchRegion { feature, region } => selection::sketch_regions(evaluation, feature)
            .map(|regions| region_points(document, evaluation, feature, regions, region))
            .unwrap_or_default(),
    }
}

fn region_triangles(mesh: &RegionMesh, plane: &Plane) -> Vec<[Point3; 3]> {
    mesh.triangles
        .iter()
        .filter_map(|triangle| {
            let [a, b, c] = triangle.map(|index| {
                mesh.points
                    .get(index as usize)
                    .map(|point| plane.to_world(*point))
            });
            Some([a?, b?, c?])
        })
        .collect()
}

fn region_points(
    document: &Document,
    evaluation: &Evaluation,
    sketch: FeatureId,
    regions: &[SketchRegion],
    region: RegionKey,
) -> Vec<Point3> {
    let Some(plane) = sketch_plane(document, evaluation, sketch) else {
        return Vec::new();
    };
    regions
        .iter()
        .filter(|candidate| candidate.region.key() == region)
        .filter_map(|candidate| candidate.mesh.as_ref())
        .flat_map(|mesh| mesh.points.iter().map(|point| plane.to_world(*point)))
        .collect()
}

fn model_bounds(sources: &Sources<'_>) -> Option<Aabb> {
    let Sources {
        document,
        evaluation,
        bodies,
        sketches,
        ..
    } = *sources;
    let sketches = document
        .active_features()
        .filter(|feature| !feature.hidden)
        .filter_map(|feature| sketches.bounds(evaluation, feature, sketch_points_bounds))
        .reduce(Aabb::union);
    let shown_bodies = bodies
        .iter()
        .filter(|(body, _)| visibility::is_shown(document, *body))
        .filter_map(|(_, mesh)| mesh.bounds())
        .reduce(Aabb::union);
    match (sketches, shown_bodies) {
        (Some(sketches), Some(bodies)) => Some(sketches.union(bodies)),
        (Some(bounds), None) | (None, Some(bounds)) => Some(bounds),
        (None, None) => None,
    }
}

fn reference_size(model: Option<Aabb>) -> f64 {
    model
        .map(|bounds| {
            let reach = bounds.min().abs().max(bounds.max().abs());
            reach.max_element() * REFERENCE_MARGIN
        })
        .unwrap_or(0.0)
        .max(MIN_REFERENCE_SIZE)
}

fn axis_ends(ray: Ray, near: Point3, size: f64) -> [Point3; 2] {
    let middle = (near - ray.origin()).dot(ray.direction());
    [ray.at(middle - size), ray.at(middle + size)]
}

fn datum_plane_corners(plane: Plane, size: f64) -> [Point3; 4] {
    let centre = plane.origin() - plane.normal() * plane.signed_distance(Point3::ZERO);
    let centred = Plane::from_frame(centre, plane.normal(), plane.x_axis()).unwrap_or(plane);
    plane_corners(centred, size * DATUM_PLANE_SCALE)
}

fn datum_points(evaluation: &Evaluation, feature: FeatureId, size: f64) -> Vec<Point3> {
    match datum_tools::result(evaluation, feature) {
        Some(DatumResult::Plane(plane)) => datum_plane_corners(plane, size).to_vec(),
        Some(DatumResult::Axis(ray)) => axis_ends(ray, Point3::ZERO, size).to_vec(),
        Some(DatumResult::Point(point)) => vec![point],
        None => Vec::new(),
    }
}

fn plane_corners(plane: Plane, size: f64) -> [Point3; 4] {
    [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
        .map(|(u, v)| plane.to_world(Point2::new(u * size, v * size)))
}

#[cfg(test)]
mod tests {
    use caditor_document::{CancelToken, FeatureKind, ModelEvaluator, Recompute};
    use caditor_expression::{Expression, Unit};
    use caditor_geometry::Plane;

    use super::*;
    use crate::scene_palette::{HIGH_CONTRAST, STANDARD};

    fn document() -> (Document, FeatureId, EntityId) {
        let mut document = Document::default();
        let mut sketch = Sketch::new(Plane::XY);
        let line = sketch.add_line(Point2::ZERO, Point2::new(40.0, 0.0));
        let mut transaction = document.transaction("Add sketch");
        let feature = transaction.add_feature("Base sketch", FeatureKind::from(sketch));
        document.apply(transaction.finish()).unwrap();
        (document, feature, line)
    }

    const FACETING: f64 = 0.01;

    fn build_for(
        document: &Document,
        evaluation: &Evaluation,
        highlight: &Highlight<'_>,
        editing: Option<FeatureId>,
    ) -> BuiltScene {
        build_in(
            Contrast::default(),
            document,
            evaluation,
            highlight,
            editing,
        )
    }

    fn build_in(
        contrast: Contrast,
        document: &Document,
        evaluation: &Evaluation,
        highlight: &Highlight<'_>,
        editing: Option<FeatureId>,
    ) -> BuiltScene {
        build(
            &Sources {
                document,
                evaluation,
                bodies: &BodyMeshes::default(),
                sketches: &DisplayedSketches::default(),
                style: DisplayStyle::default(),
                contrast,
            },
            highlight,
            Context {
                sketch: editing,
                ..Context::default()
            },
            &mut SketchShapes::new(Faceting::within(FACETING)),
        )
    }

    impl BuiltScene {
        fn bounds_with(
            &self,
            document: &Document,
            evaluation: &Evaluation,
            pickables: impl IntoIterator<Item = Pickable>,
        ) -> Option<Aabb> {
            self.bounds_of(
                &Sources {
                    document,
                    evaluation,
                    bodies: &BodyMeshes::default(),
                    sketches: &DisplayedSketches::default(),
                    style: DisplayStyle::default(),
                    contrast: Contrast::default(),
                },
                pickables,
            )
        }
    }

    fn hit(id: PickId, offset_points: f32) -> PickHit {
        PickHit {
            id,
            offset_points,
            position: Point3::ZERO,
        }
    }

    #[test]
    fn every_pickable_gets_a_unique_id_and_sketch_geometry_is_on_its_plane() {
        let (document, feature, line) = document();
        let selection = Selection::default();
        let built = build_for(
            &document,
            &Evaluation::default(),
            &Highlight {
                selection: &selection,
                hovered: &[],
            },
            None,
        );

        assert_eq!(built.picks.entries.len(), 3 + 3 + 1 + 3);
        let line_pick = built
            .picks
            .entries
            .iter()
            .position(|(pickable, _)| {
                *pickable
                    == Pickable::SketchEntity {
                        feature,
                        entity: line,
                    }
            })
            .and_then(PickId::from_index);
        let drawn = built
            .scene
            .lines()
            .find(|drawn| drawn.pick == line_pick)
            .unwrap();
        assert_eq!(drawn.end, Point3::new(40.0, 0.0, 0.0));
        assert_eq!(built.everything.max().x, 48.0);
        assert_eq!(built.fit_all().max().x, 40.0);

        let empty = build_for(
            &Document::default(),
            &Evaluation::default(),
            &Highlight {
                selection: &selection,
                hovered: &[],
            },
            None,
        );
        assert_eq!(empty.model, None);
        assert_eq!(empty.fit_all(), empty.everything);
    }

    fn evaluate(document: &Document) -> Evaluation {
        Recompute::default().run(document, &ModelEvaluator, &CancelToken::never(), &|_, _| {})
    }

    fn picks_of(built: &BuiltScene) -> Vec<Pickable> {
        built
            .picks
            .entries
            .iter()
            .map(|(pickable, _)| *pickable)
            .collect()
    }

    fn line_color(built: &BuiltScene, pickable: Pickable) -> Color {
        let pick = built
            .picks
            .entries
            .iter()
            .position(|(candidate, _)| *candidate == pickable)
            .and_then(PickId::from_index);
        built
            .scene
            .lines()
            .find(|line| line.pick == pick)
            .unwrap()
            .color
    }

    #[test]
    fn sketches_tint_their_closed_regions_once_found_and_pick_them_outside_editing() {
        let mut document = Document::default();
        let mut sketch = Sketch::new(Plane::XY);
        let corners = [
            Point2::ZERO,
            Point2::new(10.0, 0.0),
            Point2::new(10.0, 5.0),
            Point2::new(0.0, 5.0),
        ];
        for index in 0..4 {
            sketch.add_line(corners[index], corners[(index + 1) % 4]);
        }
        sketch.add_line(Point2::new(20.0, 0.0), Point2::new(30.0, 0.0));
        let mut transaction = document.transaction("Add sketch");
        let feature = transaction.add_feature("Outline", FeatureKind::from(sketch));
        document.apply(transaction.finish()).unwrap();
        let evaluation = evaluate(&document);
        let selection = Selection::default();
        let highlight = Highlight {
            selection: &selection,
            hovered: &[],
        };

        let editing = build_for(&document, &evaluation, &highlight, Some(feature));
        let looking = build_for(&document, &evaluation, &highlight, None);

        let tinted: Vec<&Fill> = editing
            .scene
            .fills()
            .filter(|fill| fill.color == STANDARD.closed_region)
            .collect();
        assert_eq!(tinted.len(), 1);
        let area: f64 = tinted[0]
            .triangles
            .iter()
            .map(|[a, b, c]| (*b - *a).cross(*c - *a).length() / 2.0)
            .sum();
        assert!((area - 50.0).abs() < 1e-9, "{area}");
        assert!(tinted.iter().all(|fill| fill.pick.is_none()));
        let shown: Vec<&Fill> = looking
            .scene
            .fills()
            .filter(|fill| fill.color == STANDARD.closed_region)
            .collect();
        assert_eq!(shown.len(), 1);
        assert_eq!(shown[0].layer, Layer::Model);
        assert!(shown[0].pick.is_some());
    }

    #[test]
    fn editing_shows_only_the_edited_sketch_and_its_references_as_pickable() {
        let (mut document, base, line) = document();
        let mut transaction = document.transaction("Add sketch");
        let mut other = Sketch::new(Plane::XZ);
        other.add_line(Point2::ZERO, Point2::new(0.0, 10.0));
        let side = transaction.add_feature("Side", FeatureKind::from(other));
        document.apply(transaction.finish()).unwrap();
        let selection = Selection::default();
        let highlight = Highlight {
            selection: &selection,
            hovered: &[],
        };

        let built = build_for(&document, &Evaluation::default(), &highlight, Some(side));

        let references: Vec<Pickable> = [
            EntityId::HORIZONTAL_AXIS,
            EntityId::VERTICAL_AXIS,
            EntityId::ORIGIN,
        ]
        .into_iter()
        .map(|entity| Pickable::SketchEntity {
            feature: side,
            entity,
        })
        .collect();
        let picks = picks_of(&built);
        assert_eq!(picks.len(), 3 + 3);
        assert_eq!(picks[..3], references[..]);
        assert!(picks.iter().all(|pickable| matches!(
            pickable,
            Pickable::SketchEntity { feature, .. } if *feature == side
        )));
        assert_eq!(built.scene.fills().count(), 0);
        assert_eq!(built.scene.grid.as_ref().unwrap().plane, Plane::XZ);
        let background = built
            .scene
            .lines()
            .find(|drawn| drawn.end == Point3::new(40.0, 0.0, 0.0))
            .unwrap();
        assert_eq!(background.pick, None);
        assert_eq!(background.color, STANDARD.background.curve);
        let edited = built.edited.unwrap();
        assert_eq!(edited.feature, side);
        assert_eq!(edited.bounds.max(), Point3::new(0.0, 0.0, 10.0));
        assert_eq!(built.fit_all(), edited.bounds);
        assert!(
            built
                .bounds_with(&document, &Evaluation::default(), [references[0]])
                .is_some_and(|bounds| bounds.max().x > 0.0 && bounds.min().x < 0.0)
        );
        assert!(!picks.contains(&Pickable::SketchEntity {
            feature: base,
            entity: line
        }));
    }

    fn is_part_of(built: &BuiltScene, pick: Option<PickId>, sketch: FeatureId) -> bool {
        pick.and_then(|id| built.picks.resolve(id))
            .is_some_and(|(pickable, _)| {
                matches!(pickable, Pickable::SketchEntity { feature, .. } if feature == sketch)
            })
    }

    #[test]
    fn the_edited_sketch_its_references_and_the_preview_draw_in_front_of_everything_else() {
        let (mut document, base, line) = document();
        let mut transaction = document.transaction("Add sketch");
        let mut other = Sketch::new(Plane::XZ);
        other.add_line(Point2::ZERO, Point2::new(0.0, 10.0));
        let side = transaction.add_feature("Side", FeatureKind::from(other));
        document.apply(transaction.finish()).unwrap();
        let selection = Selection::default();
        let hovered = [Pickable::SketchEntity {
            feature: side,
            entity: EntityId::ORIGIN,
        }];
        let highlight = Highlight {
            selection: &selection,
            hovered: &hovered,
        };

        let editing = build_for(&document, &Evaluation::default(), &highlight, Some(side));
        let viewing = build_for(&document, &Evaluation::default(), &highlight, None);

        let in_front = |layer: Layer| layer == Layer::Front;
        assert!(editing.scene.lines().any(|drawn| !in_front(drawn.layer)));
        assert!(editing.scene.markers().any(|drawn| !in_front(drawn.layer)));
        for drawn in editing.scene.lines() {
            assert_eq!(
                in_front(drawn.layer),
                is_part_of(&editing, drawn.pick, side),
                "{drawn:?}"
            );
        }
        for drawn in editing.scene.markers() {
            assert_eq!(
                in_front(drawn.layer),
                is_part_of(&editing, drawn.pick, side),
                "{drawn:?}"
            );
        }
        let origin = editing.picks.id_of(hovered[0]);
        let highlighted = editing
            .scene
            .markers()
            .find(|drawn| drawn.pick == origin)
            .unwrap();
        assert_eq!(highlighted.color, STANDARD.lines.hovered);
        assert_eq!(highlighted.layer, Layer::Front);
        let base_line = viewing.picks.id_of(Pickable::SketchEntity {
            feature: base,
            entity: line,
        });
        assert!(viewing.scene.lines().any(|drawn| drawn.pick == base_line));
        assert!(viewing.scene.lines().all(|drawn| !in_front(drawn.layer)));
        assert!(viewing.scene.markers().all(|drawn| !in_front(drawn.layer)));
        assert!(viewing.scene.fills().all(|drawn| !in_front(drawn.layer)));

        let mut preview = Batch::default();
        add_preview(
            &mut preview,
            &STANDARD,
            Plane::XZ,
            &Preview {
                curves: vec![vec![Point2::ZERO, Point2::new(5.0, 5.0)]],
                removed: vec![vec![Point2::new(5.0, 5.0), Point2::new(9.0, 5.0)]],
                points: vec![Point2::new(5.0, 5.0)],
                snap: Some(Point2::ZERO),
                guides: vec![[Point2::ZERO, Point2::new(0.0, 9.0)]],
                construction: false,
            },
        );
        assert_eq!((preview.lines.len(), preview.markers.len()), (3, 2));
        assert!(matches!(
            preview.lines.last().map(|drawn| drawn.stroke),
            Some(Stroke::Dashed { .. })
        ));
        assert!(preview.lines.iter().all(|drawn| in_front(drawn.layer)));
        assert!(preview.markers.iter().all(|drawn| in_front(drawn.layer)));
    }

    #[test]
    fn sketch_geometry_is_colored_by_its_constraint_state() {
        let mut document = Document::default();
        let mut sketch = Sketch::new(Plane::XY);
        let fixed = sketch.add_line(Point2::ZERO, Point2::new(40.0, 0.0));
        let Some(&Entity::Line { start, end }) = sketch.entity(fixed) else {
            panic!("expected a line");
        };
        sketch
            .add_constraint(Constraint::Coincident(start, EntityId::ORIGIN))
            .unwrap();
        sketch
            .add_constraint(Constraint::Horizontal(fixed))
            .unwrap();
        sketch
            .add_constraint(Constraint::Distance {
                from: start,
                to: end,
                value: Expression::Measure(40.0, Unit::Millimetre),
            })
            .unwrap();
        let free = sketch.add_line(Point2::new(0.0, 10.0), Point2::new(5.0, 20.0));
        let doubled = sketch.add_line(Point2::new(0.0, -10.0), Point2::new(5.0, -10.0));
        sketch
            .add_constraint(Constraint::Horizontal(doubled))
            .unwrap();
        sketch
            .add_constraint(Constraint::Horizontal(doubled))
            .unwrap();
        let mut transaction = document.transaction("Add sketch");
        let feature = transaction.add_feature("States", FeatureKind::from(sketch));
        document.apply(transaction.finish()).unwrap();
        let selection = Selection::default();
        let highlight = Highlight {
            selection: &selection,
            hovered: &[],
        };
        let entity = |entity| Pickable::SketchEntity { feature, entity };

        let built = build_for(&document, &evaluate(&document), &highlight, Some(feature));
        assert_eq!(
            line_color(&built, entity(fixed)),
            STANDARD.fully_constrained.curve
        );
        assert_eq!(
            line_color(&built, entity(free)),
            STANDARD.under_constrained.curve
        );
        assert_eq!(
            line_color(&built, entity(doubled)),
            STANDARD.redundant.curve
        );

        let mut transaction = document.transaction("Conflict");
        transaction.add_sketch_constraint(feature, Constraint::Vertical(fixed));
        document.apply(transaction.finish()).unwrap();
        let built = build_for(&document, &evaluate(&document), &highlight, Some(feature));
        assert_eq!(
            line_color(&built, entity(fixed)),
            STANDARD.conflicting.curve
        );
        assert_eq!(
            line_color(&built, entity(free)),
            STANDARD.under_constrained.curve
        );

        let selection = Selection::default();
        let hovered = Highlight {
            selection: &selection,
            hovered: &[entity(fixed)],
        };
        let built = build_for(&document, &evaluate(&document), &hovered, Some(feature));
        assert_eq!(line_color(&built, entity(fixed)), STANDARD.lines.hovered);
    }

    fn line_width(built: &BuiltScene, pickable: Pickable) -> f32 {
        let pick = built.picks.id_of(pickable);
        built
            .scene
            .lines()
            .find(|line| line.pick == pick)
            .unwrap()
            .width
    }

    fn hollow(built: &BuiltScene, pickable: Pickable) -> bool {
        let pick = built.picks.id_of(pickable);
        let marker = built
            .scene
            .markers()
            .find(|marker| marker.pick == pick)
            .unwrap();
        built.scene.markers().any(|hole| {
            hole.pick.is_none()
                && hole.position == marker.position
                && hole.diameter < marker.diameter
                && hole.color == HIGH_CONTRAST.hole
        })
    }

    #[test]
    fn high_contrast_draws_constrained_geometry_heavier_and_free_points_hollow() {
        let mut document = Document::default();
        let mut sketch = Sketch::new(Plane::XY);
        let fixed = sketch.add_line(Point2::ZERO, Point2::new(40.0, 0.0));
        let Some(&Entity::Line { start, end }) = sketch.entity(fixed) else {
            panic!("expected a line");
        };
        sketch
            .add_constraint(Constraint::Coincident(start, EntityId::ORIGIN))
            .unwrap();
        sketch
            .add_constraint(Constraint::Horizontal(fixed))
            .unwrap();
        sketch
            .add_constraint(Constraint::Distance {
                from: start,
                to: end,
                value: Expression::Measure(40.0, Unit::Millimetre),
            })
            .unwrap();
        let free = sketch.add_line(Point2::new(0.0, 10.0), Point2::new(5.0, 20.0));
        let Some(&Entity::Line {
            start: loose_start,
            end: loose_end,
        }) = sketch.entity(free)
        else {
            panic!("expected a line");
        };
        let mut transaction = document.transaction("Add sketch");
        let feature = transaction.add_feature("States", FeatureKind::from(sketch));
        document.apply(transaction.finish()).unwrap();
        let evaluation = evaluate(&document);
        let selection = Selection::default();
        let highlight = Highlight {
            selection: &selection,
            hovered: &[],
        };
        let entity = |entity| Pickable::SketchEntity { feature, entity };

        let high = build_in(
            Contrast::High,
            &document,
            &evaluation,
            &highlight,
            Some(feature),
        );
        let standard = build_for(&document, &evaluation, &highlight, Some(feature));

        assert!(line_width(&high, entity(fixed)) > line_width(&high, entity(free)));
        assert_eq!(
            line_color(&high, entity(fixed)),
            HIGH_CONTRAST.fully_constrained.curve
        );
        for point in [loose_start, loose_end] {
            assert!(hollow(&high, entity(point)), "{point:?}");
        }
        for point in [start, end] {
            assert!(!hollow(&high, entity(point)), "{point:?}");
        }
        assert_eq!(
            line_width(&standard, entity(fixed)),
            line_width(&standard, entity(free))
        );
        assert!(standard.scene.markers().all(|marker| marker.pick.is_some()));
    }

    #[test]
    fn points_win_over_curves_and_surfaces_within_tolerance() {
        let mut picks = PickTable::default();
        let plane = picks
            .register(Pickable::Plane(PrincipalPlane::Xy), PickPriority::Surface)
            .unwrap();
        let axis = picks
            .register(Pickable::Axis(Axis::X), PickPriority::Curve)
            .unwrap();
        let origin = picks
            .register(Pickable::Origin, PickPriority::Point)
            .unwrap();
        let result = |hits| PickResult {
            cursor: caditor_geometry::Vector2::ZERO,
            hits,
        };

        let best = picks.best_hit(
            &result(vec![hit(plane, 0.0), hit(axis, 1.0), hit(origin, 6.0)]),
            SelectionFilter::Everything,
        );
        assert_eq!(best.map(|(pickable, _)| pickable), Some(Pickable::Origin));

        let best = picks.best_hit(
            &result(vec![hit(plane, 0.0), hit(axis, 6.0)]),
            SelectionFilter::Everything,
        );
        assert_eq!(
            best.map(|(pickable, _)| pickable),
            Some(Pickable::Plane(PrincipalPlane::Xy))
        );

        assert_eq!(
            picks.best_hit(&result(vec![hit(plane, 2.0)]), SelectionFilter::Everything),
            None
        );
        assert_eq!(
            picks.best_hit(
                &result(vec![hit(plane, 0.0), hit(axis, 1.0), hit(origin, 6.0)]),
                SelectionFilter::Faces
            ),
            None
        );
    }

    #[test]
    fn high_contrast_widens_selected_geometry_more_than_hovered_geometry() {
        let mut document = Document::default();
        let mut sketch = Sketch::new(Plane::XY);
        let chosen = sketch.add_line(Point2::ZERO, Point2::new(40.0, 0.0));
        let pointed = sketch.add_line(Point2::new(0.0, 10.0), Point2::new(40.0, 10.0));
        let mut transaction = document.transaction("Add sketch");
        let feature = transaction.add_feature("Lines", FeatureKind::from(sketch));
        document.apply(transaction.finish()).unwrap();
        let entity = |entity| Pickable::SketchEntity { feature, entity };
        let mut selection = Selection::default();
        selection.replace_with(entity(chosen));
        let hovered = [entity(pointed)];
        let highlight = Highlight {
            selection: &selection,
            hovered: &hovered,
        };

        let high = build_in(
            Contrast::High,
            &document,
            &Evaluation::default(),
            &highlight,
            Some(feature),
        );
        let standard = build_for(&document, &Evaluation::default(), &highlight, Some(feature));

        assert!(line_width(&high, entity(chosen)) > line_width(&high, entity(pointed)));
        assert_eq!(
            line_width(&standard, entity(chosen)),
            line_width(&standard, entity(pointed))
        );
        assert_ne!(
            HIGH_CONTRAST.troubled_edges_dashed,
            STANDARD.troubled_edges_dashed
        );
    }

    #[test]
    fn highlight_prefers_selection_colors() {
        let mut selection = Selection::default();
        selection.replace_with(Pickable::Origin);
        let highlight = Highlight {
            selection: &selection,
            hovered: &[Pickable::Origin],
        };
        assert_eq!(
            highlight.color(&STANDARD.lines, Pickable::Origin, STANDARD.origin),
            STANDARD.lines.hovered_selected
        );
        assert_eq!(
            highlight.color(&STANDARD.lines, Pickable::Axis(Axis::Z), STANDARD.origin),
            STANDARD.origin
        );
    }

    #[test]
    fn selection_bounds_cover_the_selected_line() {
        let (document, feature, line) = document();
        let selection = Selection::default();
        let built = build_for(
            &document,
            &Evaluation::default(),
            &Highlight {
                selection: &selection,
                hovered: &[],
            },
            None,
        );
        let bounds = built
            .bounds_with(
                &document,
                &Evaluation::default(),
                [Pickable::SketchEntity {
                    feature,
                    entity: line,
                }],
            )
            .unwrap();
        assert_eq!(bounds.min(), Point3::ZERO);
        assert_eq!(bounds.max(), Point3::new(40.0, 0.0, 0.0));
    }

    #[test]
    fn curves_are_drawn_as_polylines_that_share_one_pick() {
        let mut document = Document::default();
        let mut sketch = Sketch::new(Plane::XY);
        let circle = sketch.add_circle(Point2::new(10.0, 0.0), 30.0);
        let arc = sketch.add_arc(Point2::ZERO, Point2::new(5.0, 0.0), Point2::new(0.0, 5.0));
        let spline =
            sketch.add_spline(&[Point2::ZERO, Point2::new(3.0, 6.0), Point2::new(8.0, 1.0)]);
        let mut transaction = document.transaction("Add sketch");
        let feature = transaction.add_feature("Curves", FeatureKind::from(sketch));
        document.apply(transaction.finish()).unwrap();
        let selection = Selection::default();
        let built = build_for(
            &document,
            &Evaluation::default(),
            &Highlight {
                selection: &selection,
                hovered: &[],
            },
            None,
        );

        for entity in [circle, arc, spline] {
            let pickable = Pickable::SketchEntity { feature, entity };
            let registered: Vec<_> = built
                .picks
                .entries
                .iter()
                .enumerate()
                .filter(|(_, (candidate, _))| *candidate == pickable)
                .collect();
            assert_eq!(registered.len(), 1);
            let (index, (_, priority)) = registered[0];
            assert_eq!(*priority, PickPriority::Curve);
            let pick = PickId::from_index(index);
            let segments = built.scene.lines().filter(|line| line.pick == pick).count();
            assert!(segments > 4, "only {segments} segments");
        }
        assert_eq!(built.everything.max().x, 48.0);
        assert_eq!(built.everything.min().x, -48.0);
        let bounds = built
            .bounds_with(
                &document,
                &Evaluation::default(),
                [Pickable::SketchEntity {
                    feature,
                    entity: circle,
                }],
            )
            .unwrap();
        assert!((bounds.max().x - 40.0).abs() < 1e-9);
        assert!((bounds.min().y + 30.0).abs() < 1e-9);
    }

    fn circles(radii: impl IntoIterator<Item = f64>) -> (Document, FeatureId, Vec<EntityId>) {
        let mut sketch = Sketch::new(Plane::XY);
        let circles = radii
            .into_iter()
            .enumerate()
            .map(|(index, radius)| sketch.add_circle(Point2::new(index as f64 * 5.0, 0.0), radius))
            .collect();
        let mut document = Document::default();
        let mut transaction = document.transaction("Add sketch");
        let feature = transaction.add_feature("Circles", FeatureKind::from(sketch));
        document.apply(transaction.finish()).unwrap();
        (document, feature, circles)
    }

    fn segments_of(built: &BuiltScene, pickable: Pickable) -> usize {
        let pick = built.picks.id_of(pickable);
        built
            .scene
            .lines()
            .filter(|line| line.pick.is_some() && line.pick == pick)
            .count()
    }

    #[test]
    fn sketch_curves_are_faceted_to_the_chord_tolerance_they_are_given() {
        let (document, feature, circles) = circles([1.0, 100.0]);
        let selection = Selection::default();
        let highlight = Highlight {
            selection: &selection,
            hovered: &[],
        };
        let sources = Sources {
            document: &document,
            evaluation: &Evaluation::default(),
            bodies: &BodyMeshes::default(),
            sketches: &DisplayedSketches::default(),
            style: DisplayStyle::default(),
            contrast: Contrast::default(),
        };
        let built_at = |chord: f64| {
            build(
                &sources,
                &highlight,
                Context::default(),
                &mut SketchShapes::new(Faceting::within(chord)),
            )
        };
        let [small, large] =
            [circles[0], circles[1]].map(|entity| Pickable::SketchEntity { feature, entity });

        let coarse = built_at(1.0);
        let fine = built_at(0.01);

        assert_eq!(segments_of(&coarse, small), 12);
        assert_eq!(
            segments_of(&coarse, large),
            Faceting::within(1.0).arc_segments(100.0, std::f64::consts::TAU)
        );
        assert_eq!(
            segments_of(&fine, large),
            Faceting::within(0.01).arc_segments(100.0, std::f64::consts::TAU)
        );
        assert!(segments_of(&coarse, large) > segments_of(&coarse, small));
        assert!(segments_of(&fine, small) > segments_of(&coarse, small));
        assert!(segments_of(&fine, large) > segments_of(&coarse, large) * 9);
    }

    #[test]
    fn sketches_needing_more_segments_than_the_budget_are_faceted_more_coarsely() {
        let (document, _, _) = circles(std::iter::repeat_n(50.0, 700));
        let (few, _, _) = circles([50.0]);
        let evaluation = Evaluation::default();
        let bodies = BodyMeshes::default();
        let sketches = DisplayedSketches::default();
        let sources = |document| Sources {
            document,
            evaluation: &evaluation,
            bodies: &bodies,
            sketches: &sketches,
            style: DisplayStyle::default(),
            contrast: Contrast::default(),
        };
        let wanted = Faceting::within(1e-9);

        let crowded = drawn_faceting(&sources(&document), Context::default(), wanted);
        let alone = drawn_faceting(&sources(&few), Context::default(), wanted);
        let selection = Selection::default();
        let built = build(
            &sources(&document),
            &Highlight {
                selection: &selection,
                hovered: &[],
            },
            Context::default(),
            &mut SketchShapes::new(crowded),
        );
        let drawn = built
            .scene
            .lines()
            .filter(|line| line.pick.is_some())
            .count();

        assert_eq!(alone, wanted);
        assert!(crowded.chord() > wanted.chord());
        assert!(drawn <= SKETCH_SEGMENT_BUDGET, "{drawn}");
        assert!(drawn > SKETCH_SEGMENT_BUDGET / 4, "{drawn}");
    }
}
