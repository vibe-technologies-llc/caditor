use caditor_document::{Document, FeatureId, FeatureKind};
use caditor_geometry::{Aabb, Plane, Point2, Point3};
use caditor_render::{Color, Fill, Grid, Layer, Line, Marker, PickHit, PickId, PickResult, Scene};
use caditor_sketch::{Entity, EntityId, Sketch};

use crate::selection::{Axis, Pickable, PrincipalPlane, Selection};

const MIN_REFERENCE_SIZE: f64 = 20.0;
const REFERENCE_MARGIN: f64 = 1.2;

const GRID: Color = Color::from_rgba8(210, 215, 225, 90);
const SKETCH_CURVE: Color = Color::from_rgb8(222, 224, 230);
const SKETCH_POINT: Color = Color::from_rgb8(245, 245, 248);
const ORIGIN: Color = Color::from_rgb8(235, 235, 235);
const PLANE_FILL: Color = Color::from_rgba8(120, 150, 200, 22);
const PLANE_EDGE: Color = Color::from_rgba8(140, 170, 215, 150);
const HOVERED: Color = Color::from_rgb8(255, 196, 84);
const SELECTED: Color = Color::from_rgb8(86, 170, 255);
const HOVERED_SELECTED: Color = Color::from_rgb8(150, 205, 255);
const HIGHLIGHT_FILL_ALPHA: f32 = 0.22;

const CURVE_WIDTH: f32 = 2.0;
const AXIS_WIDTH: f32 = 2.0;
const PLANE_EDGE_WIDTH: f32 = 1.25;
const HIGHLIGHT_EXTRA_WIDTH: f32 = 1.5;
const POINT_DIAMETER: f32 = 7.0;
const ORIGIN_DIAMETER: f32 = 8.0;
const HIGHLIGHT_EXTRA_DIAMETER: f32 = 3.0;

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
    pub hovered: Option<Pickable>,
}

impl Highlight<'_> {
    fn color(&self, pickable: Pickable, base: Color) -> Color {
        let hovered = self.hovered == Some(pickable);
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
        if self.hovered == Some(pickable) || self.selection.contains(pickable) {
            1.0
        } else {
            0.0
        }
    }
}

pub struct BuiltScene {
    pub scene: Scene,
    pub picks: PickTable,
    pub everything: Aabb,
    reference_size: f64,
}

impl BuiltScene {
    pub fn bounds_of<'a>(
        &self,
        document: &Document,
        pickables: impl IntoIterator<Item = Pickable> + 'a,
    ) -> Option<Aabb> {
        Aabb::from_points(
            pickables
                .into_iter()
                .flat_map(|pickable| pickable_points(document, pickable, self.reference_size)),
        )
    }
}

pub fn build(document: &Document, highlight: &Highlight<'_>) -> BuiltScene {
    let model = model_bounds(document);
    let reference_size = reference_size(model);
    let mut builder = Builder {
        scene: Scene {
            grid: Some(Grid {
                plane: Plane::XY,
                color: GRID,
            }),
            ..Scene::default()
        },
        picks: PickTable::default(),
        highlight,
    };

    for plane in PrincipalPlane::ALL {
        builder.principal_plane(plane, reference_size);
    }
    for axis in Axis::ALL {
        builder.axis(axis, reference_size);
    }
    builder.origin();
    for feature in document.features() {
        match &feature.kind {
            FeatureKind::Sketch(sketch) => builder.sketch(feature.id(), sketch),
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
        scene: builder.scene,
        picks: builder.picks,
        everything,
        reference_size,
    }
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
        self.scene.fills.push(Fill {
            convex_outline: corners.to_vec(),
            color: self.highlight.fill_color(pickable, PLANE_FILL),
            pick,
        });
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

    fn sketch(&mut self, feature: FeatureId, sketch: &Sketch) {
        let plane = sketch.plane();
        for (entity, kind) in sketch.entities() {
            let pickable = Pickable::SketchEntity { feature, entity };
            let emphasis = self.highlight.emphasis(pickable);
            match kind {
                Entity::Point(position) => self.scene.markers.push(Marker {
                    position: plane.to_world(*position),
                    color: self.highlight.color(pickable, SKETCH_POINT),
                    diameter: POINT_DIAMETER + emphasis * HIGHLIGHT_EXTRA_DIAMETER,
                    layer: Layer::Model,
                    pick: self.picks.register(pickable, PickPriority::Point),
                }),
                Entity::Line { .. } => {
                    let Some((start, end)) = sketch.line_endpoints(entity) else {
                        continue;
                    };
                    self.scene.lines.push(Line {
                        start: plane.to_world(start),
                        end: plane.to_world(end),
                        color: self.highlight.color(pickable, SKETCH_CURVE),
                        width: CURVE_WIDTH + emphasis * HIGHLIGHT_EXTRA_WIDTH,
                        layer: Layer::Model,
                        pick: self.picks.register(pickable, PickPriority::Curve),
                    });
                }
            }
        }
    }
}

fn sketch_entity_points(sketch: &Sketch, entity: EntityId) -> Vec<Point3> {
    let plane = sketch.plane();
    match sketch.entity(entity) {
        Some(Entity::Point(position)) => vec![plane.to_world(*position)],
        Some(Entity::Line { .. }) => sketch
            .line_endpoints(entity)
            .map(|(start, end)| vec![plane.to_world(start), plane.to_world(end)])
            .unwrap_or_default(),
        None => Vec::new(),
    }
}

fn pickable_points(document: &Document, pickable: Pickable, reference_size: f64) -> Vec<Point3> {
    match pickable {
        Pickable::Origin => vec![Point3::ZERO],
        Pickable::Axis(axis) => vec![Point3::ZERO, axis.direction() * reference_size],
        Pickable::Plane(plane) => plane_corners(plane.plane(), reference_size).to_vec(),
        Pickable::SketchEntity { feature, entity } => document
            .feature(feature)
            .map(|owner| match &owner.kind {
                FeatureKind::Sketch(sketch) => sketch_entity_points(sketch, entity),
            })
            .unwrap_or_default(),
    }
}

fn model_bounds(document: &Document) -> Option<Aabb> {
    Aabb::from_points(document.features().iter().flat_map(|feature| {
        match &feature.kind {
            FeatureKind::Sketch(sketch) => sketch
                .entities()
                .flat_map(|(entity, _)| sketch_entity_points(sketch, entity))
                .collect::<Vec<_>>(),
        }
    }))
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
    use caditor_geometry::Plane;

    use super::*;

    fn document() -> (Document, FeatureId, EntityId) {
        let mut document = Document::default();
        let mut sketch = Sketch::new(Plane::XY);
        let line = sketch.add_line(Point2::ZERO, Point2::new(40.0, 0.0));
        let feature = document.add_feature("Base sketch", FeatureKind::Sketch(sketch));
        (document, feature, line)
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
        let built = build(
            &document,
            &Highlight {
                selection: &selection,
                hovered: None,
            },
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
            hovered: Some(Pickable::Origin),
        };
        assert_eq!(highlight.color(Pickable::Origin, ORIGIN), HOVERED_SELECTED);
        assert_eq!(highlight.color(Pickable::Axis(Axis::Z), ORIGIN), ORIGIN);
    }

    #[test]
    fn selection_bounds_cover_the_selected_line() {
        let (document, feature, line) = document();
        let selection = Selection::default();
        let built = build(
            &document,
            &Highlight {
                selection: &selection,
                hovered: None,
            },
        );
        let bounds = built
            .bounds_of(
                &document,
                [Pickable::SketchEntity {
                    feature,
                    entity: line,
                }],
            )
            .unwrap();
        assert_eq!(bounds.min(), Point3::ZERO);
        assert_eq!(bounds.max(), Point3::new(40.0, 0.0, 0.0));
    }
}
