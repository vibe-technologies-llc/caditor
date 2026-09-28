use std::{borrow::Cow, collections::BTreeSet};

use caditor_document::{
    Document, Evaluation, Feature, FeatureId, FeatureResult, FeatureState, RegionChoice,
    SketchRegion, SolidFeature,
};
use caditor_geometry::{Aabb, Plane, Point2, Point3};
use caditor_kernel::RegionKey;
use caditor_render::{
    Color, FaceStyle, Fill, Grid, Layer, Line, Marker, MeshInstance, PickHit, PickId, PickResult,
    Scene,
};
use caditor_sketch::{
    Constraint, ConstraintId, Entity, EntityId, EntityState, Reference, Sketch, SketchSolution,
};

use crate::{
    bodies::{BodyMesh, BodyMeshes},
    drawing::Preview,
    editing::Context,
    selection::{self, Axis, Pickable, PrincipalPlane, Selection},
};

const MIN_REFERENCE_SIZE: f64 = 20.0;
const EMPTY_SKETCH_HALF_SIZE: f64 = 50.0;
const CURVE_SEGMENT_ANGLE: f64 = std::f64::consts::PI / 60.0;
const REFERENCE_MARGIN: f64 = 1.2;

const GRID: Color = Color::from_rgba8(210, 215, 225, 90);
const SKETCH_CURVE: Color = Color::from_rgb8(222, 224, 230);
const SKETCH_POINT: Color = Color::from_rgb8(245, 245, 248);
const FAILED_SKETCH_CURVE: Color = Color::from_rgb8(214, 120, 110);
const FAILED_SKETCH_POINT: Color = Color::from_rgb8(232, 146, 136);
const FULLY_CONSTRAINED_CURVE: Color = Color::from_rgb8(104, 204, 120);
const FULLY_CONSTRAINED_POINT: Color = Color::from_rgb8(146, 226, 158);
const CONFLICTING_CURVE: Color = Color::from_rgb8(238, 78, 70);
const CONFLICTING_POINT: Color = Color::from_rgb8(250, 108, 100);
const REDUNDANT_CURVE: Color = Color::from_rgb8(236, 132, 40);
const REDUNDANT_POINT: Color = Color::from_rgb8(246, 158, 78);
const BACKGROUND_SKETCH_CURVE: Color = Color::from_rgb8(104, 108, 118);
const BACKGROUND_SKETCH_POINT: Color = Color::from_rgb8(118, 122, 132);
const SKETCH_HORIZONTAL_AXIS: Color = Color::from_rgb8(226, 84, 84);
const SKETCH_VERTICAL_AXIS: Color = Color::from_rgb8(112, 196, 88);
const ORIGIN: Color = Color::from_rgb8(235, 235, 235);
const PLANE_FILL: Color = Color::from_rgba8(120, 150, 200, 22);
const PLANE_EDGE: Color = Color::from_rgba8(140, 170, 215, 150);
const HOVERED: Color = Color::from_rgb8(255, 196, 84);
const SELECTED: Color = Color::from_rgb8(86, 170, 255);
const HOVERED_SELECTED: Color = Color::from_rgb8(150, 205, 255);
const HIGHLIGHT_FILL_ALPHA: f32 = 0.22;
const PREVIEW_CURVE: Color = Color::from_rgb8(190, 150, 255);
const PREVIEW_POINT: Color = Color::from_rgb8(214, 190, 255);
const SNAP_MARKER: Color = Color::from_rgb8(80, 226, 236);
const BODY: Color = Color::from_rgb8(150, 162, 180);
const FAILED_BODY: Color = Color::from_rgb8(200, 134, 124);
const OUTDATED_BODY: Color = Color::from_rgb8(182, 170, 130);
const BACKGROUND_BODY: Color = Color::from_rgb8(92, 96, 104);
const BODY_EDGE: Color = Color::from_rgb8(30, 32, 38);
const BACKGROUND_BODY_EDGE: Color = Color::from_rgb8(62, 64, 70);
const CHOSEN_REGION: Color = Color::from_rgba8(86, 170, 255, 90);
const OPEN_REGION: Color = Color::from_rgba8(210, 214, 224, 26);
const HOVERED_REGION_ALPHA: f32 = 0.4;
const REVOLVE_AXIS: Color = Color::from_rgb8(255, 150, 60);

const CURVE_WIDTH: f32 = 2.0;
const BODY_EDGE_WIDTH: f32 = 1.5;
const REVOLVE_AXIS_WIDTH: f32 = 2.5;
const AXIS_WIDTH: f32 = 2.0;
const SKETCH_AXIS_WIDTH: f32 = 1.5;
const PLANE_EDGE_WIDTH: f32 = 1.25;
const HIGHLIGHT_EXTRA_WIDTH: f32 = 1.5;
const POINT_DIAMETER: f32 = 7.0;
const ORIGIN_DIAMETER: f32 = 8.0;
const HIGHLIGHT_EXTRA_DIAMETER: f32 = 3.0;
const SNAP_MARKER_DIAMETER: f32 = 13.0;

#[derive(Debug, Clone, Copy, PartialEq)]
struct Palette {
    curve: Color,
    point: Color,
}

const UNDER_CONSTRAINED: Palette = Palette {
    curve: SKETCH_CURVE,
    point: SKETCH_POINT,
};
const FULLY_CONSTRAINED: Palette = Palette {
    curve: FULLY_CONSTRAINED_CURVE,
    point: FULLY_CONSTRAINED_POINT,
};
const CONFLICTING: Palette = Palette {
    curve: CONFLICTING_CURVE,
    point: CONFLICTING_POINT,
};
const REDUNDANT: Palette = Palette {
    curve: REDUNDANT_CURVE,
    point: REDUNDANT_POINT,
};
const FAILED: Palette = Palette {
    curve: FAILED_SKETCH_CURVE,
    point: FAILED_SKETCH_POINT,
};
const BACKGROUND: Palette = Palette {
    curve: BACKGROUND_SKETCH_CURVE,
    point: BACKGROUND_SKETCH_POINT,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PickPriority {
    Point,
    Curve,
    Surface,
}

impl PickPriority {
    fn tolerance_px(self) -> f32 {
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

    #[cfg(test)]
    pub fn pickables(&self) -> impl Iterator<Item = Pickable> + '_ {
        self.entries.iter().map(|(pickable, _)| *pickable)
    }

    pub fn best_hit(&self, result: &PickResult) -> Option<(Pickable, PickHit)> {
        result
            .hits
            .iter()
            .filter_map(|hit| {
                let (pickable, priority) = self.resolve(hit.id)?;
                (hit.offset_px <= priority.tolerance_px()).then_some((priority, pickable, *hit))
            })
            .min_by(|a, b| a.0.cmp(&b.0).then(a.2.offset_px.total_cmp(&b.2.offset_px)))
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

    fn color(&self, pickable: Pickable, base: Color) -> Color {
        let hovered = self.is_hovered(pickable);
        match (self.selection.contains(pickable), hovered) {
            (true, true) => HOVERED_SELECTED,
            (true, false) => SELECTED,
            (false, true) => HOVERED,
            (false, false) => base,
        }
    }

    fn fill_color(&self, pickable: Pickable, base: Color) -> Color {
        let color = self.color(pickable, base);
        if color == base {
            base
        } else {
            color.with_alpha(HIGHLIGHT_FILL_ALPHA)
        }
    }

    fn emphasis(&self, pickable: Pickable) -> f32 {
        if self.is_hovered(pickable) || self.selection.contains(pickable) {
            1.0
        } else {
            0.0
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EditedSketch {
    pub feature: FeatureId,
    pub plane: Plane,
    pub bounds: Aabb,
}

pub struct BuiltScene {
    pub scene: Scene,
    pub picks: PickTable,
    pub everything: Aabb,
    pub edited: Option<EditedSketch>,
    reference_size: f64,
}

pub struct Sources<'a> {
    pub document: &'a Document,
    pub evaluation: &'a Evaluation,
    pub bodies: &'a BodyMeshes,
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
        self.edited.map_or(self.everything, |edited| edited.bounds)
    }
}

pub fn build(sources: &Sources<'_>, highlight: &Highlight<'_>, context: Context) -> BuiltScene {
    let Sources {
        document,
        evaluation,
        bodies,
    } = *sources;
    let editing = context.sketch;
    let model = model_bounds(sources);
    let reference_size = reference_size(model);
    let edited = editing
        .and_then(|id| document.feature(id))
        .and_then(|feature| Some((feature, displayed_sketch(evaluation, feature)?)));
    let grid_plane = edited
        .as_ref()
        .map_or(Plane::XY, |(_, displayed)| displayed.plane());
    let mut builder = Builder {
        scene: Scene {
            grid: Some(Grid {
                plane: grid_plane,
                color: GRID,
            }),
            ..Scene::default()
        },
        picks: PickTable::default(),
        highlight,
    };

    match &edited {
        Some((feature, displayed)) => {
            builder.sketch_references(feature.id(), displayed.plane(), reference_size);
        }
        None => {
            for plane in PrincipalPlane::ALL {
                builder.principal_plane(plane, reference_size);
            }
            for axis in Axis::ALL {
                builder.axis(axis, reference_size);
            }
            builder.origin();
        }
    }
    for feature in document.features() {
        let presence = match editing {
            None => Presence::Normal,
            Some(edited) if edited == feature.id() => Presence::Edited,
            Some(_) => Presence::Background,
        };
        let (Some(displayed), Some(states)) = (
            displayed_sketch(evaluation, feature),
            ConstraintStates::of(evaluation, feature),
        ) else {
            continue;
        };
        builder.sketch(feature.id(), &displayed, &states, presence);
    }
    for (body, mesh) in bodies.iter() {
        let color = match editing {
            Some(_) => None,
            None => Some(body_color(document, evaluation, body)),
        };
        builder.body(body, mesh, color);
    }
    if let Some(feature) = context.solid {
        builder.swept(document, evaluation, feature, reference_size);
    }

    let reference = Aabb::from_points(plane_corners(Plane::XY, reference_size))
        .map(|bounds| bounds.including(Point3::Z * reference_size));
    let everything = match (model, reference) {
        (Some(model), Some(reference)) => model.union(reference),
        (Some(bounds), None) | (None, Some(bounds)) => bounds,
        (None, None) => Aabb::from_point(Point3::ZERO),
    };

    BuiltScene {
        scene: builder.scene,
        picks: builder.picks,
        everything,
        edited: edited.map(|(feature, displayed)| EditedSketch {
            feature: feature.id(),
            plane: displayed.plane(),
            bounds: sketch_bounds(&displayed),
        }),
        reference_size,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Presence {
    Normal,
    Edited,
    Background,
}

struct ConstraintStates<'a> {
    solution: Option<&'a SketchSolution>,
    conflicting: BTreeSet<EntityId>,
    redundant: BTreeSet<EntityId>,
    failed: bool,
}

impl<'a> ConstraintStates<'a> {
    fn of(evaluation: &'a Evaluation, feature: &Feature) -> Option<Self> {
        let definition = feature.kind.sketch()?;
        let mut states = Self {
            solution: None,
            conflicting: BTreeSet::new(),
            redundant: BTreeSet::new(),
            failed: false,
        };
        let Some(status) = evaluation.feature(feature.id()) else {
            return Some(states);
        };
        match &status.state {
            FeatureState::Failed(error) if !error.constraints.is_empty() => {
                states.conflicting = entities_of(definition, &error.constraints);
            }
            FeatureState::Failed(_) | FeatureState::Outdated => states.failed = true,
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

    fn palette(&self, entity: EntityId) -> Palette {
        if self.conflicting.contains(&entity) {
            CONFLICTING
        } else if self.failed {
            FAILED
        } else if self.redundant.contains(&entity) {
            REDUNDANT
        } else if self
            .solution
            .and_then(|solution| solution.entity_state(entity))
            == Some(EntityState::FullyConstrained)
        {
            FULLY_CONSTRAINED
        } else {
            UNDER_CONSTRAINED
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
    scene: Scene,
    picks: PickTable,
    highlight: &'a Highlight<'a>,
}

impl Builder<'_> {
    fn principal_plane(&mut self, plane: PrincipalPlane, size: f64) {
        let pickable = Pickable::Plane(plane);
        let pick = self.picks.register(pickable, PickPriority::Surface);
        let corners = plane_corners(plane.plane(), size);
        let edge = self.highlight.color(pickable, PLANE_EDGE);
        let width = PLANE_EDGE_WIDTH + self.highlight.emphasis(pickable) * HIGHLIGHT_EXTRA_WIDTH;
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
            });
        }
        self.scene.fills.push(Fill::convex(
            &corners,
            self.highlight.fill_color(pickable, PLANE_FILL),
            Layer::Reference,
            pick,
        ));
    }

    fn body(&mut self, body: FeatureId, mesh: &BodyMesh, color: Option<Color>) {
        let faces = mesh
            .faces
            .iter()
            .map(|face| match color {
                Some(base) => {
                    let pickable = Pickable::Face {
                        body,
                        face: face.key,
                    };
                    FaceStyle {
                        color: self.highlight.color(pickable, base),
                        pick: self.picks.register(pickable, PickPriority::Surface),
                    }
                }
                None => FaceStyle {
                    color: BACKGROUND_BODY,
                    pick: None,
                },
            })
            .collect();
        self.scene.meshes.push(MeshInstance {
            mesh: std::sync::Arc::clone(&mesh.mesh),
            faces,
        });
        for edge in &mesh.edges {
            let pickable = Pickable::Edge {
                body,
                edge: edge.name,
            };
            let (color, width, pick) = match color {
                Some(_) => (
                    self.highlight.color(pickable, BODY_EDGE),
                    BODY_EDGE_WIDTH + self.highlight.emphasis(pickable) * HIGHLIGHT_EXTRA_WIDTH,
                    self.picks.register(pickable, PickPriority::Curve),
                ),
                None => (BACKGROUND_BODY_EDGE, BODY_EDGE_WIDTH, None),
            };
            let segments = edge.points.windows(2).filter_map(|pair| match pair {
                [start, end] => Some(Line {
                    start: *start,
                    end: *end,
                    color,
                    width,
                    layer: Layer::Model,
                    pick,
                }),
                _ => None,
            });
            self.scene.lines.extend(segments);
        }
    }

    fn swept(
        &mut self,
        document: &Document,
        evaluation: &Evaluation,
        feature: FeatureId,
        reference_size: f64,
    ) {
        let Some(solid) = document
            .feature(feature)
            .and_then(|owner| owner.kind.solid())
        else {
            return;
        };
        let Some(plane) = document
            .feature(solid.sketch())
            .and_then(|sketch| sketch.kind.sketch())
            .map(Sketch::plane)
        else {
            return;
        };
        if let SolidFeature::Revolve(revolve) = solid {
            let displayed = document
                .feature(solid.sketch())
                .and_then(|sketch| displayed_sketch(evaluation, sketch));
            let ends = match revolve.axis.reference() {
                Some(reference) => Some(reference_points(plane, reference, reference_size)),
                None => displayed
                    .as_deref()
                    .and_then(|sketch| sketch.line_endpoints(revolve.axis))
                    .map(|(start, end)| [plane.to_world(start), plane.to_world(end)]),
            };
            if let Some([start, end]) = ends {
                self.scene.lines.push(Line {
                    start,
                    end,
                    color: REVOLVE_AXIS,
                    width: REVOLVE_AXIS_WIDTH,
                    layer: Layer::Model,
                    pick: None,
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
            let color = if self.highlight.is_hovered(pickable) {
                HOVERED.with_alpha(HOVERED_REGION_ALPHA)
            } else if chosen.contains(&key) {
                CHOSEN_REGION
            } else {
                OPEN_REGION
            };
            let triangles = mesh
                .triangles
                .iter()
                .filter_map(|triangle| {
                    let [a, b, c] = triangle.map(|index| {
                        mesh.points
                            .get(index as usize)
                            .map(|point| plane.to_world(*point))
                    });
                    Some([a?, b?, c?])
                })
                .collect();
            self.scene.fills.push(Fill {
                triangles,
                color,
                layer: Layer::Model,
                pick: self.picks.register(pickable, PickPriority::Surface),
            });
        }
    }

    fn axis(&mut self, axis: Axis, size: f64) {
        let pickable = Pickable::Axis(axis);
        let [red, green, blue] = axis.rgb();
        let base = Color::from_rgb8(red, green, blue);
        self.scene.lines.push(Line {
            start: Point3::ZERO,
            end: axis.direction() * size,
            color: self.highlight.color(pickable, base),
            width: AXIS_WIDTH + self.highlight.emphasis(pickable) * HIGHLIGHT_EXTRA_WIDTH,
            layer: Layer::Reference,
            pick: self.picks.register(pickable, PickPriority::Curve),
        });
    }

    fn origin(&mut self) {
        let pickable = Pickable::Origin;
        self.scene.markers.push(Marker {
            position: Point3::ZERO,
            color: self.highlight.color(pickable, ORIGIN),
            diameter: ORIGIN_DIAMETER
                + self.highlight.emphasis(pickable) * HIGHLIGHT_EXTRA_DIAMETER,
            layer: Layer::Reference,
            pick: self.picks.register(pickable, PickPriority::Point),
        });
    }

    fn sketch_references(&mut self, feature: FeatureId, plane: Plane, size: f64) {
        for (reference, color) in [
            (Reference::HorizontalAxis, SKETCH_HORIZONTAL_AXIS),
            (Reference::VerticalAxis, SKETCH_VERTICAL_AXIS),
        ] {
            let pickable = Pickable::SketchEntity {
                feature,
                entity: reference.id(),
            };
            let [start, end] = reference_points(plane, reference, size);
            self.scene.lines.push(Line {
                start,
                end,
                color: self.highlight.color(pickable, color),
                width: SKETCH_AXIS_WIDTH
                    + self.highlight.emphasis(pickable) * HIGHLIGHT_EXTRA_WIDTH,
                layer: Layer::Reference,
                pick: self.picks.register(pickable, PickPriority::Curve),
            });
        }
        let pickable = Pickable::SketchEntity {
            feature,
            entity: EntityId::ORIGIN,
        };
        self.scene.markers.push(Marker {
            position: plane.origin(),
            color: self.highlight.color(pickable, ORIGIN),
            diameter: ORIGIN_DIAMETER
                + self.highlight.emphasis(pickable) * HIGHLIGHT_EXTRA_DIAMETER,
            layer: Layer::Reference,
            pick: self.picks.register(pickable, PickPriority::Point),
        });
    }

    fn sketch(
        &mut self,
        feature: FeatureId,
        sketch: &Sketch,
        states: &ConstraintStates<'_>,
        presence: Presence,
    ) {
        let plane = sketch.plane();
        for (entity, kind) in sketch.entities() {
            let pickable = Pickable::SketchEntity { feature, entity };
            let (palette, emphasis, pickable) = match presence {
                Presence::Background => (BACKGROUND, 0.0, None),
                Presence::Normal | Presence::Edited => (
                    states.palette(entity),
                    self.highlight.emphasis(pickable),
                    Some(pickable),
                ),
            };
            let color = |base: Color| match pickable {
                Some(pickable) => self.highlight.color(pickable, base),
                None => base,
            };
            match kind {
                Entity::Point(position) => {
                    let color = color(palette.point);
                    self.scene.markers.push(Marker {
                        position: plane.to_world(*position),
                        color,
                        diameter: POINT_DIAMETER + emphasis * HIGHLIGHT_EXTRA_DIAMETER,
                        layer: Layer::Model,
                        pick: pickable.and_then(|pickable| {
                            self.picks.register(pickable, PickPriority::Point)
                        }),
                    });
                }
                Entity::Line { .. }
                | Entity::Circle { .. }
                | Entity::Arc { .. }
                | Entity::Spline { .. } => {
                    let Some(points) = sketch.polyline(entity, CURVE_SEGMENT_ANGLE) else {
                        continue;
                    };
                    let color = color(palette.curve);
                    let width = CURVE_WIDTH + emphasis * HIGHLIGHT_EXTRA_WIDTH;
                    let pick = pickable
                        .and_then(|pickable| self.picks.register(pickable, PickPriority::Curve));
                    let segments = points.windows(2).filter_map(|pair| match pair {
                        [start, end] => Some(Line {
                            start: plane.to_world(*start),
                            end: plane.to_world(*end),
                            color,
                            width,
                            layer: Layer::Model,
                            pick,
                        }),
                        _ => None,
                    });
                    self.scene.lines.extend(segments);
                }
            }
        }
    }
}

pub fn add_preview(scene: &mut Scene, plane: Plane, preview: &Preview) {
    for curve in &preview.curves {
        let segments = curve.windows(2).filter_map(|pair| match pair {
            [start, end] => Some(Line {
                start: plane.to_world(*start),
                end: plane.to_world(*end),
                color: PREVIEW_CURVE,
                width: CURVE_WIDTH,
                layer: Layer::Model,
                pick: None,
            }),
            _ => None,
        });
        scene.lines.extend(segments);
    }
    let snap = preview.snap.map(|position| Marker {
        position: plane.to_world(position),
        color: SNAP_MARKER,
        diameter: SNAP_MARKER_DIAMETER,
        layer: Layer::Model,
        pick: None,
    });
    let points = preview.points.iter().map(|position| Marker {
        position: plane.to_world(*position),
        color: PREVIEW_POINT,
        diameter: POINT_DIAMETER,
        layer: Layer::Model,
        pick: None,
    });
    scene.markers.extend(snap.into_iter().chain(points));
}

pub fn chosen_regions(choice: &RegionChoice, regions: &[SketchRegion]) -> BTreeSet<RegionKey> {
    match choice {
        RegionChoice::All => regions
            .iter()
            .filter(|region| region.even_depth)
            .map(|region| region.region.key())
            .collect(),
        RegionChoice::Chosen(keys) => keys.iter().copied().collect(),
    }
}

fn body_color(document: &Document, evaluation: &Evaluation, body: FeatureId) -> Color {
    let mut color = BODY;
    for feature in document
        .features()
        .filter(|feature| feature.body() == Some(body))
    {
        match evaluation.feature(feature.id()).map(|status| &status.state) {
            Some(FeatureState::Failed(_)) => return FAILED_BODY,
            Some(FeatureState::Outdated) => color = OUTDATED_BODY,
            Some(FeatureState::UpToDate) | None => {}
        }
    }
    color
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
            .polyline(entity, CURVE_SEGMENT_ANGLE)
            .unwrap_or_default()
            .into_iter()
            .map(|point| plane.to_world(point))
            .collect(),
        None => Vec::new(),
    }
}

fn sketch_bounds(sketch: &Sketch) -> Aabb {
    let plane = sketch.plane();
    Aabb::from_points(
        sketch
            .entities()
            .flat_map(|(entity, _)| sketch_entity_points(sketch, entity, 0.0)),
    )
    .or_else(|| Aabb::from_points(plane_corners(plane, EMPTY_SKETCH_HALF_SIZE)))
    .unwrap_or_else(|| Aabb::from_point(plane.origin()))
}

pub fn displayed_sketch<'a>(
    evaluation: &'a Evaluation,
    feature: &'a Feature,
) -> Option<Cow<'a, Sketch>> {
    let definition = feature.kind.sketch()?;
    let last_good = evaluation
        .feature(feature.id())
        .and_then(|status| status.result.as_deref())
        .and_then(FeatureResult::sketch)
        .map(|result| &result.geometry);
    Some(match last_good {
        Some(solved) => with_solved_positions(definition, solved),
        None => Cow::Borrowed(definition),
    })
}

fn with_solved_positions<'a>(definition: &'a Sketch, solved: &'a Sketch) -> Cow<'a, Sketch> {
    let same_entities =
        definition.entities().len() == solved.entities().len()
            && definition.entities().zip(solved.entities()).all(
                |((id, defined), (other, settled))| id == other && defined.same_structure(settled),
            );
    if same_entities && definition.plane() == solved.plane() {
        return Cow::Borrowed(solved);
    }
    let mut merged = definition.clone();
    for (id, settled) in solved.entities() {
        let fits = definition
            .entity(id)
            .is_some_and(|defined| defined.same_structure(settled));
        if fits && let Err(error) = merged.replace_entity(id, settled.clone()) {
            log::debug!("showing the drawn position of entity {id}: {error}");
        }
    }
    Cow::Owned(merged)
}

fn pickable_points(sources: &Sources<'_>, pickable: Pickable, reference_size: f64) -> Vec<Point3> {
    let Sources {
        document,
        evaluation,
        bodies,
    } = *sources;
    match pickable {
        Pickable::Origin => vec![Point3::ZERO],
        Pickable::Axis(axis) => vec![Point3::ZERO, axis.direction() * reference_size],
        Pickable::Plane(plane) => plane_corners(plane.plane(), reference_size).to_vec(),
        Pickable::SketchEntity { feature, entity } => document
            .feature(feature)
            .and_then(|owner| displayed_sketch(evaluation, owner))
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
        Pickable::Region { feature, region } => {
            let Some((sketch, regions)) = selection::swept_regions(document, evaluation, feature)
            else {
                return Vec::new();
            };
            let Some(plane) = document
                .feature(sketch)
                .and_then(|sketch| sketch.kind.sketch())
                .map(Sketch::plane)
            else {
                return Vec::new();
            };
            regions
                .iter()
                .filter(|candidate| candidate.region.key() == region)
                .filter_map(|candidate| candidate.mesh.as_ref())
                .flat_map(|mesh| mesh.points.iter().map(|point| plane.to_world(*point)))
                .collect()
        }
    }
}

fn model_bounds(sources: &Sources<'_>) -> Option<Aabb> {
    let Sources {
        document,
        evaluation,
        bodies,
    } = *sources;
    let sketches = Aabb::from_points(document.features().flat_map(|feature| {
        let Some(sketch) = displayed_sketch(evaluation, feature) else {
            return Vec::new();
        };
        sketch
            .entities()
            .flat_map(|(entity, _)| sketch_entity_points(&sketch, entity, 0.0))
            .collect::<Vec<_>>()
    }));
    match (sketches, bodies.bounds()) {
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

    fn document() -> (Document, FeatureId, EntityId) {
        let mut document = Document::default();
        let mut sketch = Sketch::new(Plane::XY);
        let line = sketch.add_line(Point2::ZERO, Point2::new(40.0, 0.0));
        let mut transaction = document.transaction("Add sketch");
        let feature = transaction.add_feature("Base sketch", FeatureKind::Sketch(sketch));
        document.apply(transaction.finish()).unwrap();
        (document, feature, line)
    }

    fn build_for(
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
            },
            highlight,
            Context {
                sketch: editing,
                solid: None,
            },
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
                },
                pickables,
            )
        }
    }

    fn hit(id: PickId, offset_px: f32) -> PickHit {
        PickHit {
            id,
            offset_px,
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
            .lines
            .iter()
            .find(|drawn| drawn.pick == line_pick)
            .unwrap();
        assert_eq!(drawn.end, Point3::new(40.0, 0.0, 0.0));
        assert_eq!(built.everything.max().x, 48.0);
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
            .lines
            .iter()
            .find(|line| line.pick == pick)
            .unwrap()
            .color
    }

    #[test]
    fn editing_shows_only_the_edited_sketch_and_its_references_as_pickable() {
        let (mut document, base, line) = document();
        let mut transaction = document.transaction("Add sketch");
        let mut other = Sketch::new(Plane::XZ);
        other.add_line(Point2::ZERO, Point2::new(0.0, 10.0));
        let side = transaction.add_feature("Side", FeatureKind::Sketch(other));
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
        assert_eq!(built.scene.fills.len(), 0);
        assert_eq!(built.scene.grid.as_ref().unwrap().plane, Plane::XZ);
        let background = built
            .scene
            .lines
            .iter()
            .find(|drawn| drawn.end == Point3::new(40.0, 0.0, 0.0))
            .unwrap();
        assert_eq!(background.pick, None);
        assert_eq!(background.color, BACKGROUND_SKETCH_CURVE);
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
        let feature = transaction.add_feature("States", FeatureKind::Sketch(sketch));
        document.apply(transaction.finish()).unwrap();
        let selection = Selection::default();
        let highlight = Highlight {
            selection: &selection,
            hovered: &[],
        };
        let entity = |entity| Pickable::SketchEntity { feature, entity };

        let built = build_for(&document, &evaluate(&document), &highlight, Some(feature));
        assert_eq!(line_color(&built, entity(fixed)), FULLY_CONSTRAINED_CURVE);
        assert_eq!(line_color(&built, entity(free)), SKETCH_CURVE);
        assert_eq!(line_color(&built, entity(doubled)), REDUNDANT_CURVE);

        let mut transaction = document.transaction("Conflict");
        transaction.add_sketch_constraint(feature, Constraint::Vertical(fixed));
        document.apply(transaction.finish()).unwrap();
        let built = build_for(&document, &evaluate(&document), &highlight, Some(feature));
        assert_eq!(line_color(&built, entity(fixed)), CONFLICTING_CURVE);
        assert_eq!(line_color(&built, entity(free)), SKETCH_CURVE);

        let selection = Selection::default();
        let hovered = Highlight {
            selection: &selection,
            hovered: &[entity(fixed)],
        };
        let built = build_for(&document, &evaluate(&document), &hovered, Some(feature));
        assert_eq!(line_color(&built, entity(fixed)), HOVERED);
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

        let best = picks.best_hit(&result(vec![
            hit(plane, 0.0),
            hit(axis, 1.0),
            hit(origin, 6.0),
        ]));
        assert_eq!(best.map(|(pickable, _)| pickable), Some(Pickable::Origin));

        let best = picks.best_hit(&result(vec![hit(plane, 0.0), hit(axis, 6.0)]));
        assert_eq!(
            best.map(|(pickable, _)| pickable),
            Some(Pickable::Plane(PrincipalPlane::Xy))
        );

        assert_eq!(picks.best_hit(&result(vec![hit(plane, 2.0)])), None);
    }

    #[test]
    fn highlight_prefers_selection_colors() {
        let mut selection = Selection::default();
        selection.replace_with(Pickable::Origin);
        let highlight = Highlight {
            selection: &selection,
            hovered: &[Pickable::Origin],
        };
        assert_eq!(highlight.color(Pickable::Origin, ORIGIN), HOVERED_SELECTED);
        assert_eq!(highlight.color(Pickable::Axis(Axis::Z), ORIGIN), ORIGIN);
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
        let feature = transaction.add_feature("Curves", FeatureKind::Sketch(sketch));
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
            let segments = built
                .scene
                .lines
                .iter()
                .filter(|line| line.pick == pick)
                .count();
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
}
