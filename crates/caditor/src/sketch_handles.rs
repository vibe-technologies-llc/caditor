use std::collections::BTreeSet;

use caditor_document::FeatureId;
use caditor_expression::Dimension;
use caditor_geometry::{Plane, Point2, Point3, Ray, Vector2, Vector3};
use caditor_render::{Batch, Color, Layer, Line, Stroke, View};
use caditor_sketch::{Drag, Entity, EntityId, Faceting, MAX_LENGTH, Sketch};
use thiserror::Error;

use crate::{
    drag_solver::DragCommand,
    drawing::Preview,
    field::{self, Expected, ExpressionError},
    model::Model,
    move_manipulator::{
        ARROW_POINTS, Arrow, END_ON, GAP_POINTS, HIT_POINTS, SHAFT_WIDTH, segment_distance,
        step_for, within,
    },
    place_handles::{draw_square, square_around, square_holds},
    scene_palette::ScenePalette,
    selection::Axis,
    sketch_drag::{
        Transform, TransformError, Transforming, circles_of, extent_centre, points_of, subject,
    },
    typed_point::{self, TypedValueError},
    units::Units,
};

const SQUARE_FROM: f64 = 0.3;
const SQUARE_TO: f64 = 0.5;
const EDGE_ON: f64 = 0.2;
const RING_REACH: f64 = 1.25;
const SCALE_REACH: f64 = 1.55;
const RING_SEGMENTS: usize = 72;
const TURN_STEP_DEGREES: f64 = 5.0;
const SCALE_STEP: f64 = 0.1;
const SMALLEST_SCALE: f64 = 0.01;
const FULL_TURN_DEGREES: f64 = 360.0;
const HALF_TURN_DEGREES: f64 = 180.0;
const SMALLEST_RADIUS: f64 = 1e-6;
const UNCHANGED_LENGTH: f64 = 1e-12;
const UNCHANGED_DEGREES: f64 = 1e-9;
const UNCHANGED_SCALE: f64 = 1e-9;
const ARROWS: [SketchGrip; 2] = [SketchGrip::AlongX, SketchGrip::AlongY];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SketchGrip {
    AlongX,
    AlongY,
    OnPlane,
    Rotate,
    Scale,
}

impl SketchGrip {
    pub fn words(self) -> &'static str {
        match self {
            Self::AlongX => "Drag to move along X, or type a distance",
            Self::AlongY => "Drag to move along Y, or type a distance",
            Self::OnPlane => "Drag to move on the sketch plane",
            Self::Rotate => "Drag to rotate about the pivot, or type an angle",
            Self::Scale => "Drag to scale about the pivot, or type a factor",
        }
    }

    pub fn is_translation(self) -> bool {
        matches!(self, Self::AlongX | Self::AlongY | Self::OnPlane)
    }

    pub fn typing_field(self) -> Option<(&'static str, &'static str, &'static str)> {
        match self {
            Self::AlongX => Some(("Move along X", "Enter: move   Esc: cancel", "a distance")),
            Self::AlongY => Some(("Move along Y", "Enter: move   Esc: cancel", "a distance")),
            Self::OnPlane => None,
            Self::Rotate => Some((
                "Rotate by",
                "Degrees unless a unit is named   Enter: rotate   Esc: cancel",
                "degrees",
            )),
            Self::Scale => Some((
                "Scale by",
                "A factor above zero   Enter: scale   Esc: cancel",
                "a factor",
            )),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Error)]
pub enum SketchHandleTypingError {
    #[error(transparent)]
    Expression(#[from] ExpressionError),
    #[error("{}", field::sentence(&.0.to_string()))]
    Unevaluable(caditor_expression::EvalError),
    #[error(transparent)]
    Value(#[from] TypedValueError),
    #[error(transparent)]
    Transform(#[from] TransformError),
    #[error("Keep the geometry within {} m of the sketch's origin", MAX_LENGTH / 1_000.0)]
    TooFar,
    #[error("Select sketch geometry to move it")]
    NothingSelected,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SketchHandles {
    pub feature: FeatureId,
    plane: Plane,
    pivot: Point2,
    origin: Point3,
    reach: f64,
    per_point: f64,
    sight: Vector3,
    can_transform: bool,
}

impl SketchHandles {
    pub fn of(
        sketch: &Sketch,
        feature: FeatureId,
        pivot: Point2,
        can_transform: bool,
        view: &View,
        pixels_per_point: f64,
    ) -> Option<Self> {
        let plane = sketch.plane();
        let origin = plane.to_world(pivot);
        let depth = view.view_depth(origin);

        if !view.is_drawn_at(depth) {
            return None;
        }

        let per_point = pixels_per_point * view.units_per_pixel_at(depth);
        let reach = ARROW_POINTS * per_point;
        (reach.is_finite() && reach > 0.0).then_some(Self {
            feature,
            plane,
            pivot,
            origin,
            reach,
            per_point,
            sight: view.sight_at(origin),
            can_transform,
        })
    }

    pub fn step(&self) -> f64 {
        step_for(self.per_point)
    }

    pub fn pivot(&self) -> Point2 {
        self.pivot
    }

    fn direction(&self, grip: SketchGrip) -> Option<Vector3> {
        match grip {
            SketchGrip::AlongX => Some(self.plane.x_axis()),
            SketchGrip::AlongY => Some(self.plane.y_axis()),
            SketchGrip::OnPlane | SketchGrip::Rotate | SketchGrip::Scale => None,
        }
    }

    fn arrow(&self, grip: SketchGrip) -> Option<(Point3, Point3, Vector3)> {
        let direction = self.direction(grip)?;
        (direction.dot(self.sight).abs() < END_ON).then(|| {
            (
                self.origin + direction * GAP_POINTS * self.per_point,
                self.origin + direction * self.reach,
                direction,
            )
        })
    }

    fn plane_visible(&self) -> bool {
        self.plane.normal().dot(self.sight).abs() >= EDGE_ON
    }

    fn square(&self) -> Option<[Point3; 4]> {
        if !self.plane_visible() {
            return None;
        }

        let (x, y) = (self.plane.x_axis(), self.plane.y_axis());

        let corner = |a: f64, b: f64| self.origin + x * (a * self.reach) + y * (b * self.reach);
        Some([
            corner(SQUARE_FROM, SQUARE_FROM),
            corner(SQUARE_TO, SQUARE_FROM),
            corner(SQUARE_TO, SQUARE_TO),
            corner(SQUARE_FROM, SQUARE_TO),
        ])
    }

    fn ring(&self) -> Option<Vec<Point3>> {
        if !self.can_transform || !self.plane_visible() {
            return None;
        }

        let (x, y) = (self.plane.x_axis(), self.plane.y_axis());
        let radius = RING_REACH * self.reach;
        Some(
            (0..=RING_SEGMENTS)
                .map(|step| {
                    let angle = std::f64::consts::TAU * step as f64 / RING_SEGMENTS as f64;
                    self.origin + (x * angle.cos() + y * angle.sin()) * radius
                })
                .collect(),
        )
    }

    fn scale_stem(&self) -> Option<(Point3, Point3)> {
        if !self.can_transform || !self.plane_visible() {
            return None;
        }

        let diagonal = (self.plane.x_axis() + self.plane.y_axis()).try_normalize()?;
        Some((
            self.origin + diagonal * (RING_REACH * self.reach),
            self.origin + diagonal * (SCALE_REACH * self.reach),
        ))
    }

    fn scale_square(&self) -> Option<[Point3; 4]> {
        let (_, tip) = self.scale_stem()?;
        square_around(&self.plane, tip, self.per_point, self.sight)
    }

    pub fn hit(&self, view: &View, cursor: Vector2, pixels_per_point: f64) -> Option<SketchGrip> {
        if self
            .scale_square()
            .is_some_and(|corners| square_holds(&corners, view, cursor))
        {
            return Some(SketchGrip::Scale);
        }

        let reach = HIT_POINTS * pixels_per_point;
        let arrows = ARROWS.into_iter().filter_map(|grip| {
            let (from, to, _) = self.arrow(grip)?;

            let distance = segment_distance(view.project(from)?, view.project(to)?, cursor);
            Some((distance, grip))
        });
        let stem = self.scale_stem().and_then(|(from, to)| {
            let distance = segment_distance(view.project(from)?, view.project(to)?, cursor);
            Some((distance, SketchGrip::Scale))
        });
        let ring = self.ring().and_then(|points| {
            let projected: Option<Vec<Vector2>> = points
                .into_iter()
                .map(|point| view.project(point))
                .collect();
            let distance = projected?
                .windows(2)
                .filter_map(|pair| match pair {
                    [from, to] => Some(segment_distance(*from, *to, cursor)),
                    _ => None,
                })
                .fold(f64::INFINITY, f64::min);
            Some((distance, SketchGrip::Rotate))
        });

        let nearest = arrows
            .chain(stem)
            .chain(ring)
            .filter(|(distance, _)| *distance <= reach)
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, grip)| grip);

        if nearest.is_some() {
            return nearest;
        }
        self.square()
            .is_some_and(|corners| {
                let projected: Option<Vec<Vector2>> =
                    corners.iter().map(|corner| view.project(*corner)).collect();
                projected.is_some_and(|projected| within(&projected, cursor))
            })
            .then_some(SketchGrip::OnPlane)
    }

    #[cfg(test)]
    pub fn grip(&self, grip: SketchGrip, along: f64) -> Option<Point3> {
        match grip {
            SketchGrip::AlongX | SketchGrip::AlongY => {
                let (from, tip, direction) = self.arrow(grip)?;
                Some(from.lerp(tip, 0.6) + direction * along)
            }
            SketchGrip::OnPlane => {
                let corners = self.square()?;
                Some(corners[0].lerp(corners[2], 0.5))
            }
            SketchGrip::Rotate => {
                let ring = self.ring()?;
                let index = (along.max(0.0) as usize).min(ring.len().saturating_sub(1));
                ring.get(index).copied()
            }
            SketchGrip::Scale => self.scale_stem().map(|(_, tip)| tip),
        }
    }

    pub fn drawn(&self, highlighted: Option<SketchGrip>) -> Drawn {
        Drawn {
            handles: *self,
            highlighted,
        }
    }

    fn colour(
        &self,
        palette: &ScenePalette,
        grip: SketchGrip,
        highlighted: Option<SketchGrip>,
    ) -> Color {
        if highlighted == Some(grip) {
            return palette.handle_highlighted;
        }
        match grip {
            SketchGrip::AlongX => palette.axis(Axis::X),
            SketchGrip::AlongY => palette.axis(Axis::Y),
            SketchGrip::Rotate => palette.axis(Axis::Z),
            SketchGrip::OnPlane | SketchGrip::Scale => palette.handle,
        }
    }

    pub fn add_to(
        &self,
        batch: &mut Batch,
        palette: &ScenePalette,
        highlighted: Option<SketchGrip>,
    ) {
        if let Some(corners) = self.square() {
            draw_square(
                batch,
                &corners,
                self.colour(palette, SketchGrip::OnPlane, highlighted),
            );
        }
        if let Some(ring) = self.ring() {
            let colour = self.colour(palette, SketchGrip::Rotate, highlighted);
            for pair in ring.windows(2) {
                if let [start, end] = pair {
                    batch.lines.push(Line {
                        start: *start,
                        end: *end,
                        color: colour,
                        width: SHAFT_WIDTH,
                        layer: Layer::Front,
                        pick: None,
                        stroke: Stroke::Solid,
                    });
                }
            }
        }
        if let Some((start, tip)) = self.scale_stem() {
            let colour = self.colour(palette, SketchGrip::Scale, highlighted);

            batch.lines.push(Line {
                start,
                end: tip,
                color: colour,
                width: SHAFT_WIDTH,
                layer: Layer::Front,
                pick: None,
                stroke: Stroke::Solid,
            });
            if let Some(corners) = self.scale_square() {
                draw_square(batch, &corners, colour);
            }
        }
        for grip in ARROWS {
            let Some((from, tip, direction)) = self.arrow(grip) else {
                continue;
            };

            let arrow = Arrow {
                from,
                tip,
                direction,
                per_point: self.per_point,
                forward: self.sight,
            };

            arrow.add_to(batch, self.colour(palette, grip, highlighted));
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Drawn {
    handles: SketchHandles,
    highlighted: Option<SketchGrip>,
}

impl Drawn {
    pub fn add_to(&self, batch: &mut Batch, palette: &ScenePalette) {
        self.handles.add_to(batch, palette, self.highlighted);
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SketchHandleDrag {
    pub feature: FeatureId,
    pub grip: SketchGrip,
    plane: Plane,
    pivot: Point2,
    grabbed: Point2,
    step: f64,
    label: String,
    points: Vec<(EntityId, Point2)>,
    circles: Vec<(EntityId, f64)>,
    offset: Vector2,
    last_degrees: f64,
    swept: f64,
    degrees: f64,
    scale: f64,
    sent: Option<Vec<Drag>>,
}

impl SketchHandleDrag {
    pub fn begin(
        sketch: &Sketch,
        handles: &SketchHandles,
        grip: SketchGrip,
        selected: &[EntityId],
        ray: Ray,
    ) -> Option<Self> {
        let grabbed = plane_hit(&handles.plane, ray)?;
        let (moved, pivot) = selection_targets(sketch, selected, grip, handles.pivot)?;
        let points = points_of(sketch, &moved);

        if points.is_empty() {
            return None;
        }

        let circles = circles_of(sketch, &moved);
        let verb = match grip {
            SketchGrip::AlongX | SketchGrip::AlongY | SketchGrip::OnPlane => "Move",
            SketchGrip::Rotate => "Rotate",
            SketchGrip::Scale => "Scale",
        };
        let last_degrees = degrees_of(grabbed - pivot).unwrap_or_default();
        Some(Self {
            feature: handles.feature,
            grip,
            plane: handles.plane,
            pivot,
            grabbed,
            step: handles.step(),
            label: format!("{verb} {}", subject(sketch, &moved)),
            points,
            circles,
            offset: Vector2::ZERO,
            last_degrees,
            swept: 0.0,
            degrees: 0.0,
            scale: 1.0,
            sent: None,
        })
    }

    pub fn current_pivot(&self) -> Point2 {
        if self.grip.is_translation() {
            self.pivot + self.offset
        } else {
            self.pivot
        }
    }

    pub fn follow(&mut self, ray: Ray, free: bool) -> Option<DragCommand> {
        let pointed = plane_hit(&self.plane, ray)?;
        let drags = match self.grip {
            SketchGrip::AlongX => {
                let raw = pointed.x - self.grabbed.x;
                let dx = step_value(raw, self.step, free);
                let offset = Vector2::new(dx, 0.0);
                let drags = translated_checked(&self.points, offset)?;

                self.offset = offset;
                drags
            }
            SketchGrip::AlongY => {
                let raw = pointed.y - self.grabbed.y;
                let dy = step_value(raw, self.step, free);
                let offset = Vector2::new(0.0, dy);
                let drags = translated_checked(&self.points, offset)?;

                self.offset = offset;
                drags
            }
            SketchGrip::OnPlane => {
                let raw = pointed - self.grabbed;
                let offset = Vector2::new(
                    step_value(raw.x, self.step, free),
                    step_value(raw.y, self.step, free),
                );
                let drags = translated_checked(&self.points, offset)?;

                self.offset = offset;
                drags
            }
            SketchGrip::Rotate => {
                let angle = degrees_of(pointed - self.pivot)?;

                self.swept += wrapped(angle - self.last_degrees);
                self.last_degrees = angle;

                let degrees = step_value(self.swept, TURN_STEP_DEGREES, free);
                let drags = rotated_checked(&self.points, self.pivot, degrees)?;

                self.degrees = degrees;
                drags
            }
            SketchGrip::Scale => {
                let base = (self.grabbed - self.pivot).length();

                if base <= SMALLEST_RADIUS {
                    return None;
                }

                let raw = ((pointed - self.pivot).length() / base).max(SMALLEST_SCALE);
                let factor = step_value(raw, SCALE_STEP, free).max(SMALLEST_SCALE);
                let drags = scaled_checked(&self.points, &self.circles, self.pivot, factor)?;

                self.scale = factor;
                drags
            }
        };

        if self.sent.as_ref() == Some(&drags) {
            return None;
        }
        self.sent = Some(drags.clone());
        Some(DragCommand::Move {
            feature: self.feature,
            label: self.label.clone(),
            drags,
        })
    }

    pub fn has_moved(&self) -> bool {
        match self.grip {
            SketchGrip::AlongX | SketchGrip::AlongY | SketchGrip::OnPlane => {
                self.offset.length() > UNCHANGED_LENGTH
            }
            SketchGrip::Rotate => self.degrees.abs() > UNCHANGED_DEGREES,
            SketchGrip::Scale => (self.scale - 1.0).abs() > UNCHANGED_SCALE,
        }
    }

    pub fn finish(&self) -> DragCommand {
        DragCommand::Finish { join: None }
    }

    pub fn readout(&self, units: Units) -> String {
        match self.grip {
            SketchGrip::AlongX => format!("X {}", signed_length(self.offset.x, units)),
            SketchGrip::AlongY => format!("Y {}", signed_length(self.offset.y, units)),
            SketchGrip::OnPlane => format!(
                "X {}   Y {}",
                signed_length(self.offset.x, units),
                signed_length(self.offset.y, units)
            ),
            SketchGrip::Rotate => {
                let sign = if self.degrees >= 0.0 { "+" } else { "" };
                format!("Rotate {sign}{}", units.angle_readout(self.degrees))
            }
            SketchGrip::Scale => format!("Scale {:.2}×", self.scale),
        }
    }
}

pub fn pivot_and_transformable(
    sketch: &Sketch,
    selected: &[EntityId],
    explicit_pivot: Option<Point2>,
) -> Option<(Point2, bool)> {
    let can_transform =
        Transforming::of(sketch, FeatureId::from_raw(0), selected, Transform::Rotate).is_ok();

    if let Some(pivot) = explicit_pivot {
        return Some((pivot, can_transform));
    }

    let entities = movable_entities(sketch, selected);
    let points = points_of(sketch, &entities);

    if points.is_empty() {
        return None;
    }

    let circles = circles_of(sketch, &entities);
    let pivot = Transforming::of(sketch, FeatureId::from_raw(0), selected, Transform::Rotate)
        .map(|transforming| transforming.pivot)
        .unwrap_or_else(|_| extent_centre(sketch, &points, &circles));
    Some((pivot, can_transform))
}

pub fn apply_typed(
    model: &Model,
    sketch: &Sketch,
    feature: FeatureId,
    grip: SketchGrip,
    selected: &[EntityId],
    text: &str,
) -> Result<Vec<DragCommand>, SketchHandleTypingError> {
    match grip {
        SketchGrip::AlongX | SketchGrip::AlongY => {
            let value = parse_length(model, text)?;
            let offset = if grip == SketchGrip::AlongX {
                Vector2::new(value, 0.0)
            } else {
                Vector2::new(0.0, value)
            };
            let entities = movable_entities(sketch, selected);
            let points = points_of(sketch, &entities);

            if points.is_empty() {
                return Err(SketchHandleTypingError::NothingSelected);
            }

            let drags =
                translated_checked(&points, offset).ok_or(SketchHandleTypingError::TooFar)?;
            Ok(vec![
                DragCommand::Move {
                    feature,
                    label: format!("Move {}", subject(sketch, &entities)),
                    drags,
                },
                DragCommand::Finish { join: None },
            ])
        }
        SketchGrip::Rotate => {
            let degrees = typed_point::parse_transform(model, text, Transform::Rotate)?;
            let transforming = Transforming::of(sketch, feature, selected, Transform::Rotate)?;
            Ok(transforming.rotated(degrees)?)
        }
        SketchGrip::Scale => {
            let factor = typed_point::parse_transform(model, text, Transform::Scale)?;
            let transforming = Transforming::of(sketch, feature, selected, Transform::Scale)?;
            Ok(transforming.scaled(factor)?)
        }
        SketchGrip::OnPlane => Err(SketchHandleTypingError::NothingSelected),
    }
}

pub fn preview_typed(
    model: &Model,
    sketch: &Sketch,
    (feature, grip): (FeatureId, SketchGrip),
    pivot: Point2,
    selected: &[EntityId],
    text: Option<&str>,
    faceting: Faceting,
) -> Preview {
    match grip {
        SketchGrip::AlongX | SketchGrip::AlongY => {
            let mut preview = Preview {
                snap: Some(pivot),
                ..Preview::default()
            };
            let Some(value) = text.and_then(|text| parse_length(model, text).ok()) else {
                return preview;
            };
            let offset = if grip == SketchGrip::AlongX {
                Vector2::new(value, 0.0)
            } else {
                Vector2::new(0.0, value)
            };

            preview.guides.push([pivot, pivot + offset]);
            for entity in selected {
                if let Some(points) = sketch.faceted(*entity, faceting) {
                    if points.len() >= 2 {
                        preview
                            .curves
                            .push(points.into_iter().map(|point| point + offset).collect());
                    } else if let Some(only) = points.first() {
                        preview.points.push(*only + offset);
                    }
                }
            }
            preview
        }
        SketchGrip::Rotate => Transforming::of(sketch, feature, selected, Transform::Rotate)
            .map(|transforming| {
                let amount = text.and_then(|text| {
                    typed_point::parse_transform(model, text, Transform::Rotate).ok()
                });
                transforming.preview(sketch, amount, faceting)
            })
            .unwrap_or_default(),
        SketchGrip::Scale => Transforming::of(sketch, feature, selected, Transform::Scale)
            .map(|transforming| {
                let amount = text.and_then(|text| {
                    typed_point::parse_transform(model, text, Transform::Scale).ok()
                });
                transforming.preview(sketch, amount, faceting)
            })
            .unwrap_or_default(),
        SketchGrip::OnPlane => Preview::default(),
    }
}

fn parse_length(model: &Model, text: &str) -> Result<f64, SketchHandleTypingError> {
    let parameters = model.parameters();
    let parsed = field::parse_expression(
        model.document(),
        parameters,
        text.trim(),
        Expected {
            dimension: Some(Dimension::LENGTH),
            non_negative: false,
        },
        model.units(),
    )?;

    let evaluated = parameters
        .evaluate_expression(&parsed)
        .map_err(SketchHandleTypingError::Unevaluable)?;
    Ok(evaluated.value)
}

fn movable_entities(sketch: &Sketch, selected: &[EntityId]) -> Vec<EntityId> {
    selected
        .iter()
        .copied()
        .filter(|entity| {
            !entity.is_reference()
                && (sketch.entity(*entity).is_some()
                    || sketch.text(*entity).is_some()
                    || sketch.gear(*entity).is_some())
        })
        .collect()
}

fn selection_targets(
    sketch: &Sketch,
    selected: &[EntityId],
    grip: SketchGrip,
    fallback_pivot: Point2,
) -> Option<(Vec<EntityId>, Point2)> {
    if grip.is_translation() {
        let entities = movable_entities(sketch, selected);

        return (!entities.is_empty()).then_some((entities, fallback_pivot));
    }

    let present: Vec<EntityId> = selected
        .iter()
        .copied()
        .filter(|entity| {
            sketch.entity(*entity).is_some()
                || sketch.text(*entity).is_some()
                || sketch.gear(*entity).is_some()
                || *entity == EntityId::ORIGIN
        })
        .collect();
    let curve_points: BTreeSet<EntityId> = present
        .iter()
        .flat_map(|entity| match sketch.entity(*entity) {
            Some(entity) => entity.points(),
            None => sketch
                .text(*entity)
                .map(|text| text.anchor)
                .or_else(|| sketch.gear(*entity).map(|gear| gear.centre))
                .into_iter()
                .collect(),
        })
        .collect();

    let lone_points: Vec<EntityId> = present
        .iter()
        .copied()
        .filter(|entity| {
            (*entity == EntityId::ORIGIN
                || matches!(sketch.entity(*entity), Some(Entity::Point(_))))
                && !curve_points.contains(entity)
        })
        .collect();
    let pivot_point = match lone_points.as_slice() {
        [only] if present.len() > 1 => Some(*only),
        _ => None,
    };
    let moved: Vec<EntityId> = present
        .into_iter()
        .filter(|entity| !entity.is_reference() && Some(*entity) != pivot_point)
        .collect();

    if moved.is_empty() {
        return None;
    }

    let pivot = pivot_point
        .and_then(|point| sketch.point(point))
        .unwrap_or(fallback_pivot);
    Some((moved, pivot))
}

fn plane_hit(plane: &Plane, ray: Ray) -> Option<Point2> {
    let distance = ray.intersect_plane(plane)?;
    let point = plane.to_local(ray.at(distance));
    point.is_finite().then_some(point)
}

fn degrees_of(offset: Vector2) -> Option<f64> {
    (offset.length() > SMALLEST_RADIUS).then(|| offset.y.atan2(offset.x).to_degrees())
}

fn wrapped(degrees: f64) -> f64 {
    let turned = (degrees + HALF_TURN_DEGREES).rem_euclid(FULL_TURN_DEGREES);
    turned - HALF_TURN_DEGREES
}

fn step_value(value: f64, step: f64, free: bool) -> f64 {
    if free || !(step.is_finite() && step > 0.0) {
        value
    } else {
        (value / step).round() * step
    }
}

fn translated_checked(points: &[(EntityId, Point2)], offset: Vector2) -> Option<Vec<Drag>> {
    let drags: Vec<Drag> = points
        .iter()
        .map(|(point, at)| Drag::Point {
            point: *point,
            to: *at + offset,
        })
        .collect();
    within_limits(&drags).then_some(drags)
}

fn rotated_checked(
    points: &[(EntityId, Point2)],
    pivot: Point2,
    degrees: f64,
) -> Option<Vec<Drag>> {
    let turn = Vector2::from_angle(degrees.to_radians());
    let drags: Vec<Drag> = points
        .iter()
        .map(|(point, at)| Drag::Point {
            point: *point,
            to: pivot + turn.rotate(*at - pivot),
        })
        .collect();
    within_limits(&drags).then_some(drags)
}

fn scaled_checked(
    points: &[(EntityId, Point2)],
    circles: &[(EntityId, f64)],
    pivot: Point2,
    factor: f64,
) -> Option<Vec<Drag>> {
    if !(factor.is_finite() && factor > 0.0) {
        return None;
    }

    let moved_points = points.iter().map(|(point, at)| Drag::Point {
        point: *point,
        to: pivot + (*at - pivot) * factor,
    });
    let moved_radii = circles.iter().map(|(circle, radius)| Drag::Radius {
        circle: *circle,
        to: radius * factor,
    });

    let drags: Vec<Drag> = moved_points.chain(moved_radii).collect();
    within_limits(&drags).then_some(drags)
}

fn within_limits(drags: &[Drag]) -> bool {
    drags.iter().all(|drag| match drag {
        Drag::Point { to, .. } => to.is_finite() && to.abs().max_element() <= MAX_LENGTH,
        Drag::Radius { to, .. } => to.is_finite() && *to > 0.0 && *to <= MAX_LENGTH,
    })
}

fn signed_length(value: f64, units: Units) -> String {
    let sign = if value >= 0.0 { "+" } else { "" };
    format!("{sign}{}", units.readout_text(value))
}

#[cfg(test)]
mod tests {
    use caditor_render::{Camera, Viewpoint};

    use super::*;

    fn facing_view() -> View {
        let camera = Camera::new(Viewpoint::facing(&Plane::XY, Point3::ZERO, 200.0));
        camera.view(1000.0, 800.0)
    }

    fn sketch_with_line_and_circle() -> (Sketch, EntityId, EntityId) {
        let mut sketch = Sketch::new(Plane::XY);

        let line = sketch.add_line(Point2::new(0.0, 0.0), Point2::new(20.0, 0.0));
        let circle = sketch.add_circle(Point2::new(10.0, 10.0), 5.0);
        (sketch, line, circle)
    }

    #[test]
    fn hit_testing_finds_every_grip_and_omits_transform_grips_for_a_lone_point() {
        let view = facing_view();
        let (sketch, line, _) = sketch_with_line_and_circle();
        let feature = FeatureId::from_raw(1);

        let (pivot, can_transform) = pivot_and_transformable(&sketch, &[line], None).unwrap();

        assert!(can_transform);

        let handles =
            SketchHandles::of(&sketch, feature, pivot, can_transform, &view, 1.0).unwrap();

        for grip in [
            SketchGrip::AlongX,
            SketchGrip::AlongY,
            SketchGrip::OnPlane,
            SketchGrip::Rotate,
            SketchGrip::Scale,
        ] {
            let world = handles.grip(grip, 0.0).unwrap();
            let cursor = view.project(world).unwrap();

            assert_eq!(handles.hit(&view, cursor, 1.0), Some(grip), "{grip:?}");
        }

        let mut single = Sketch::new(Plane::XY);

        let point = single.add_point(Point2::new(5.0, 5.0));
        let (lone_pivot, lone_transform) =
            pivot_and_transformable(&single, &[point], None).unwrap();

        assert!(!lone_transform);

        let lone =
            SketchHandles::of(&single, feature, lone_pivot, lone_transform, &view, 1.0).unwrap();

        assert!(lone.grip(SketchGrip::AlongX, 0.0).is_some());
        assert!(lone.grip(SketchGrip::Rotate, 0.0).is_none());
        assert!(lone.grip(SketchGrip::Scale, 0.0).is_none());
    }

    #[test]
    fn dragging_along_axes_and_plane_constrains_and_steps_translation() {
        let view = facing_view();
        let (sketch, line, _) = sketch_with_line_and_circle();
        let feature = FeatureId::from_raw(1);
        let pivot = Point2::new(10.0, 0.0);

        let handles = SketchHandles::of(&sketch, feature, pivot, true, &view, 1.0).unwrap();
        let start_ray = Ray::new(Point3::new(20.0, 0.0, 100.0), Vector3::NEG_Z).unwrap();
        let moved_ray = Ray::new(Point3::new(35.0, 40.0, 100.0), Vector3::NEG_Z).unwrap();

        let mut drag =
            SketchHandleDrag::begin(&sketch, &handles, SketchGrip::AlongX, &[line], start_ray)
                .unwrap();

        let command = drag.follow(moved_ray, true).unwrap();
        let DragCommand::Move { drags, .. } = command else {
            panic!("expected move drag command");
        };

        assert_eq!(drags.len(), 2);

        for item in drags {
            let Drag::Point { point, to } = item else {
                panic!("expected point drag");
            };
            let before = sketch.point(point).unwrap();

            assert!((to.x - (before.x + 15.0)).abs() < 1e-9);
            assert!((to.y - before.y).abs() < 1e-9);
        }

        assert!(drag.has_moved());
        assert_eq!(drag.readout(Units::default()), "X +15.00 mm");
    }

    #[test]
    fn rotate_and_scale_grips_turn_past_half_turn_and_scale_circle_radii() {
        let view = facing_view();
        let (sketch, line, circle) = sketch_with_line_and_circle();
        let feature = FeatureId::from_raw(1);

        let selected = [line, circle];
        let (pivot, can_transform) = pivot_and_transformable(&sketch, &selected, None).unwrap();

        let handles =
            SketchHandles::of(&sketch, feature, pivot, can_transform, &view, 1.0).unwrap();

        let ray_at = |x: f64, y: f64| Ray::new(Point3::new(x, y, 100.0), Vector3::NEG_Z).unwrap();
        let mut rotate = SketchHandleDrag::begin(
            &sketch,
            &handles,
            SketchGrip::Rotate,
            &selected,
            ray_at(pivot.x + 20.0, pivot.y),
        )
        .unwrap();

        rotate.follow(ray_at(pivot.x, pivot.y + 20.0), true);
        rotate.follow(ray_at(pivot.x - 20.0, pivot.y), true);
        rotate.follow(ray_at(pivot.x, pivot.y - 20.0), true);

        assert!((rotate.degrees - 270.0).abs() < 1e-6);

        let mut scale = SketchHandleDrag::begin(
            &sketch,
            &handles,
            SketchGrip::Scale,
            &selected,
            ray_at(pivot.x + 10.0, pivot.y),
        )
        .unwrap();

        let command = scale.follow(ray_at(pivot.x + 20.0, pivot.y), true).unwrap();
        let DragCommand::Move { drags, .. } = command else {
            panic!("expected scale drag command");
        };
        let radius = drags.iter().find_map(|drag| match drag {
            Drag::Radius { circle: id, to } if *id == circle => Some(*to),
            _ => None,
        });

        assert!((radius.unwrap() - 10.0).abs() < 1e-9);
        assert_eq!(scale.readout(Units::default()), "Scale 2.00×");
    }
}
